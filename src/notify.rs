use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll, Waker};
use std::cell::UnsafeCell;
use std::marker::PhantomPinned;
use std::ptr;
use std::sync::Mutex;

use crate::linked_list::{LinkedList, Node};

/// Notifies a single task to wake up.
///
/// # Examples
///
/// Usage using `tokio` as an example executor:
///
/// ```
/// use std::sync::Arc;
/// use asyncsync::Notify;
///
/// #[tokio::main]
/// async fn main() {
///     let notify = Arc::new(Notify::new());
///
///     let clone = notify.clone();
///     let handle = tokio::task::spawn(async move {
///         clone.notified().await;
///         println!("Received notification");
///     });
///
///     notify.notify_one();
///
///     handle.await.unwrap();
/// }
/// ```
#[derive(Debug)]
pub struct Notify {
    state: AtomicUsize,

    // This list is always empty when no Notified instances are alive.
    // When Notify drops, this is always empty.
    waiters: Mutex<LinkedList<UnsafeCell<Waiter>>>,
}

#[derive(Debug)]
struct Waiter {
    waker: Option<Waker>,
    notified: bool,

    _pin: PhantomPinned,
}

impl Notify {
    /// Creates a new `Notify` without any stored notification.
    pub fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
            waiters: Mutex::default(),
        }
    }

    /// Notifies all currently waiting tasks.
    ///
    /// Note that `notify_all` will only wake up all currently waiting tasks and not store any
    /// notification for future tasks. If there are no tasks waiting, `notify_all` does nothing.
    pub fn notify_all(&self) {
        self.state.store(0, Ordering::SeqCst);
        let mut waiters = self.waiters.lock().unwrap();

        for waiter in waiters.iter_mut() {
            let waiter = unsafe { &mut *waiter.get() };

            waiter.notified = true;

            if let Some(waker) = &waiter.waker {
                waker.wake_by_ref();
            }
        }
    }

    /// Notified a single waiting task.
    ///
    /// If there are no task waiting, a notification is stored and the next waiting task will
    /// complete immediately.
    pub fn notify_one(&self) {
        let waiters = self.waiters.lock().unwrap();
        if waiters.is_empty() {
            self.state.store(1, Ordering::SeqCst);
        } else {
            let waiter = unsafe { &mut *waiters.front().unwrap().get() };
            waiter.notified = true;

            if let Some(waker) = &waiter.waker {
                waker.wake_by_ref();
            }
        }
    }

    /// Wait for a notification.
    pub fn notified(&self) -> Notified<'_> {
        Notified {
            notify: self,
            state: State::Init,
            waiter: 0 as *mut _,
        }
    }
}

impl Drop for Notify {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            let waiters = self.waiters.lock().unwrap();
            assert!(waiters.is_empty());
        }
    }
}

/// A future waiting for a wake-up notification. `Notified` is returned from [`Notify::notified`].
#[derive(Debug)]
pub struct Notified<'a> {
    notify: &'a Notify,
    state: State,

    /// Pointer to the wait list of `self.notify`. Lock the mutex before accessing.
    waiter: *mut Node<UnsafeCell<Waiter>>,
}

impl<'a> Future for Notified<'a> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.state {
            State::Init => {
                let res =
                    self.notify
                        .state
                        .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst);

                if res.is_ok() {
                    self.state = State::Done;
                    return Poll::Ready(());
                }

                // Register a new waiter.
                let mut waiters = self.notify.waiters.lock().unwrap();

                let waiter = waiters.push_back(UnsafeCell::new(Waiter {
                    waker: None,
                    notified: false,
                    _pin: PhantomPinned,
                }));
                self.waiter = waiter.as_ptr();

                // SAFETY: waiters is locked.
                let waiter = unsafe { &mut *(*self.waiter).get() };
                waiter.waker = Some(cx.waker().clone());
                drop(waiter);
                drop(waiters);

                self.state = State::Pending;
                Poll::Pending
            }
            State::Pending => {
                let mut waiters = self.notify.waiters.lock().unwrap();

                // SAFETY: waiters is locked.
                let waiter = unsafe { &mut *(*self.waiter).get() };

                if waiter.notified {
                    drop(waiter);
                    let waiter = unsafe { &mut *self.waiter };

                    waiters.remove(waiter);
                    unsafe {
                        ptr::drop_in_place(self.waiter);
                    }

                    self.state = State::Done;
                    Poll::Ready(())
                } else {
                    // Update the waker if necessary.
                    let update = match &waiter.waker {
                        Some(waker) => !waker.will_wake(&cx.waker()),
                        None => true,
                    };

                    if update {
                        waiter.waker = Some(cx.waker().clone());
                    }

                    drop(waiter);
                    drop(waiters);

                    Poll::Pending
                }
            }
            State::Done => Poll::Ready(()),
        }
    }
}

impl<'a> Drop for Notified<'a> {
    fn drop(&mut self) {
        // Remove existing waiter.
        if self.state != State::Done {
            let mut waiters = self.notify.waiters.lock().unwrap();

            let node = unsafe { &mut *self.waiter };
            waiters.remove(node);

            // The waiter is only removed from the list and needs to be dropped manually.
            unsafe {
                ptr::drop_in_place(self.waiter);
            }
        }
    }
}

unsafe impl<'a> Send for Notified<'a> {}
unsafe impl<'a> Sync for Notified<'a> {}

#[derive(Debug, PartialEq)]
enum State {
    Init,
    Pending,
    Done,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::Notify;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_notify_all() {
        let notify = Arc::new(Notify::new());

        let (tx, mut rx) = mpsc::channel(5);

        for _ in 0..5 {
            let handle = notify.clone();
            let tx = tx.clone();
            tokio::task::spawn(async move {
                handle.notified().await;
                let _ = tx.send(()).await;
            });
        }

        tokio::time::sleep(Duration::new(5, 0)).await;
        notify.notify_all();

        for _ in 0..5 {
            let _ = rx.recv().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_notify_one() {
        let notify = Arc::new(Notify::new());
        notify.notify_one();

        notify.notified().await;
    }
}
