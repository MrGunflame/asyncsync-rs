use core::cell::UnsafeCell;
use core::future::Future;
use core::marker::PhantomPinned;
use core::pin::Pin;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll, Waker};

use std::sync::Mutex;

use crate::linked_list::{Link, LinkedList};

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
    waiters: Mutex<LinkedList<Waiter>>,
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
            let waiter = unsafe { waiter.get() };

            waiter.notified = true;

            if let Some(waker) = &waiter.waker {
                waker.wake_by_ref();
            }
        }
    }

    /// Notifies a single waiting task.
    ///
    /// If there are no task waiting, a notification is stored and the next waiting task will
    /// complete immediately.
    pub fn notify_one(&self) {
        let waiters = self.waiters.lock().unwrap();

        match waiters.front() {
            Some(waiter) => {
                let waiter = unsafe { waiter.get() };

                waiter.notified = true;
                if let Some(waker) = &waiter.waker {
                    waker.wake_by_ref();
                }
            }
            None => self.state.store(1, Ordering::SeqCst),
        }
    }

    /// Wait for a notification.
    pub fn notified(&self) -> Notified<'_> {
        Notified {
            notify: self,
            state: State::Init,
            waiter: Waiter(UnsafeCell::new(WaiterInner {
                waker: None,
                notified: false,
                next: None,
                prev: None,
                _pin: PhantomPinned,
            })),
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

unsafe impl Send for Notify {}
unsafe impl Sync for Notify {}

#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct Waiter(UnsafeCell<WaiterInner>);

impl Waiter {
    pub(crate) fn new() -> Self {
        Self(UnsafeCell::new(WaiterInner {
            waker: None,
            notified: false,
            _pin: PhantomPinned,
            next: None,
            prev: None,
        }))
    }

    pub(crate) unsafe fn get(&self) -> &mut WaiterInner {
        &mut *self.0.get()
    }
}

unsafe impl Link for Waiter {
    fn next(&self) -> Option<NonNull<Self>> {
        unsafe { (&*self.0.get()).next }
    }

    fn prev(&self) -> Option<NonNull<Self>> {
        unsafe { (&*self.0.get()).prev }
    }

    fn next_mut(&mut self) -> &mut Option<NonNull<Self>> {
        unsafe { &mut (&mut *self.0.get()).next }
    }

    fn prev_mut(&mut self) -> &mut Option<NonNull<Self>> {
        unsafe { &mut (&mut *self.0.get()).prev }
    }
}

#[derive(Debug)]
pub(crate) struct WaiterInner {
    pub(crate) waker: Option<Waker>,
    pub(crate) notified: bool,

    _pin: PhantomPinned,

    next: Option<NonNull<Waiter>>,
    prev: Option<NonNull<Waiter>>,
}

/// A future waiting for a wake-up notification. `Notified` is returned from [`Notify::notified`].
#[derive(Debug)]
pub struct Notified<'a> {
    notify: &'a Notify,
    state: State,

    /// Pointer to the wait list of `self.notify`. Lock the mutex before accessing. Only
    /// inside the waiterlist if state == State::Pending.
    waiter: Waiter,
}

impl<'a> Notified<'a> {
    /// Returns a `&mut self.state` from a `Pin<&mut Self>`.
    #[inline]
    fn state_mut(self: Pin<&mut Self>) -> &mut State {
        is_unpin::<State>();

        // SAFETY: State is unpin.
        unsafe { &mut self.get_unchecked_mut().state }
    }
}

impl<'a> Future for Notified<'a> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.state {
            State::Init => {
                let res =
                    self.notify
                        .state
                        .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst);

                if res.is_ok() {
                    *self.state_mut() = State::Done;
                    return Poll::Ready(());
                }

                // Lock waiters mutex before accessing `self.waiter`.
                let mut waiters = self.notify.waiters.lock().unwrap();

                // SAFETY: waiterlist is locked, access to `self.writer` is exclusive.
                unsafe {
                    self.waiter.get().waker = Some(cx.waker().clone());

                    waiters.push_back((&self.waiter).into());
                }

                drop(waiters);

                *self.state_mut() = State::Pending;
                Poll::Pending
            }
            State::Pending => {
                let mut waiters = self.notify.waiters.lock().unwrap();

                let waiter = unsafe { self.waiter.get() };

                if waiter.notified {
                    // SAFETY: Waiterlist is locked, access to `self.writer` is exclusive.
                    unsafe {
                        waiters.remove((&self.waiter).into());
                    }

                    *self.state_mut() = State::Done;
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
        // Remove existing waiter if necessary.
        if self.state == State::Pending {
            let mut waiters = self.notify.waiters.lock().unwrap();

            // SAFETY: `self.waiter` is a valid pointer in the waiterlist.
            unsafe {
                waiters.remove((&self.waiter).into());
            }
        }
    }
}

unsafe impl<'a> Send for Notified<'a> {}
unsafe impl<'a> Sync for Notified<'a> {}

#[derive(Debug, PartialEq)]
pub(crate) enum State {
    Init,
    Pending,
    Done,
}

/// A utility function that asserts whether `T`is [`Unpin`].
#[inline]
fn is_unpin<T: Unpin>() {}

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

    #[test]
    fn test_notify_notified() {
        let notify = Notify::new();
        let _: Vec<_> = (0..5).map(|_| notify.notified()).collect();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_notify_one() {
        let notify = Arc::new(Notify::new());

        let handle = notify.clone();
        tokio::task::spawn(async move {
            handle.notified().await;
        });

        tokio::time::sleep(Duration::new(1, 0)).await;
        notify.notify_one();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_notify_one_stored() {
        let notify = Arc::new(Notify::new());
        notify.notify_one();

        notify.notified().await;
    }
}
