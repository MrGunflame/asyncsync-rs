use core::cell::UnsafeCell;
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::ptr::{self, NonNull};
use core::task::{Context, Poll};

use futures::future::FusedFuture;

use crate::linked_list::LinkedList;
use crate::utils::is_unpin;
use crate::utils::notify::{State, Waiter};

/// Notifies a single task to wake up.
///
/// This is the thread-local `!Send` version of [`Notify`].
///
/// [`Notify`]: crate::Notify
#[derive(Debug)]
pub struct Notify {
    state: UnsafeCell<usize>,
    waiters: UnsafeCell<LinkedList<Waiter>>,

    _marker: PhantomData<*const ()>,
}

impl Notify {
    /// Creates a new `Notify` without any stored notification.
    #[inline]
    pub const fn new() -> Self {
        Self {
            state: UnsafeCell::new(0),
            waiters: UnsafeCell::new(LinkedList::new()),
            _marker: PhantomData,
        }
    }

    /// Notifies all currently waiting tasks.
    ///
    /// Note that `notify_all` will only wake up all currently waiting tasks and not store any
    /// notification for future tasks. If there are no tasks waiting, `notify_all` does nothing.
    pub fn notify_all(&self) {
        // Remove any remaning notifications.
        // SAFETY: No other threads have access to this field.
        // usize doesn't require dropping, overwrite it.
        unsafe {
            ptr::write(self.state.get(), 0);
        }

        // SAFETY: No other threads have access to this field.
        let waiters = unsafe { &mut *self.waiters.get() };

        for waiter in waiters.iter_mut() {
            // SAFETY: No other threads have access to this field.

            // Set the notification and wake the waker.
            waiter.notified = true;
            if let Some(waker) = &waiter.waker {
                waker.wake_by_ref();
            }
        }
    }

    /// Notifies a single waiting task.
    ///
    /// If there are not task waiting, a notification is stored and the next waiting task will
    /// complete immediately.
    pub fn notify_one(&self) {
        // SAFETY: No other threads have access to this field.
        let waiters = unsafe { &mut *self.waiters.get() };

        // Notify the first waiter in the list. If the list is empty
        // store a notification instead.
        match waiters.front_mut() {
            Some(waiter) => {
                // SAFETY: No other threads have access to this field.

                // Set the notification and wake the waker.
                waiter.notified = true;
                if let Some(waker) = &waiter.waker {
                    waker.wake_by_ref();
                }
            }
            // SAFETY: No other threads have access to this field.
            // usize doesn't required dropping, overwrite it.
            None => unsafe { ptr::write(self.state.get(), 1) },
        }
    }

    /// Wait for a notification.
    #[inline]
    pub fn notified(&self) -> Notified<'_> {
        Notified {
            notify: self,
            state: State::Init,
            waiter: UnsafeCell::new(Waiter::new()),
        }
    }
}

impl Default for Notify {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// A future waiting for a wake-up notification.
///
/// `Notified` is returned from [`Notify::notified`].
#[derive(Debug)]
pub struct Notified<'a> {
    notify: &'a Notify,
    state: State,
    waiter: UnsafeCell<Waiter>,
}

impl<'a> Notified<'a> {
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
                // If a notification is stored, take it and return immediately.
                // SAFETY: No other threads have access to this field.
                let state = unsafe { &mut *self.notify.state.get() };
                if *state == 1 {
                    *state = 0;
                    *self.state_mut() = State::Done;
                    return Poll::Ready(());
                }

                // Register the new waiter.
                // SAFETY: The waiter is owned any not has no references.
                let waiter = unsafe { &mut *self.waiter.get() };
                waiter.waker = Some(cx.waker().clone());

                // Push the new waiter.
                // SAFETY: No other threads have access to this field. The pushed waiter
                // has a shorter lifetime than the list.
                unsafe {
                    let waiters = &mut *self.notify.waiters.get();

                    let ptr = NonNull::new_unchecked(self.waiter.get());
                    waiters.push_back(ptr);
                }

                *self.state_mut() = State::Pending;
                Poll::Pending
            }
            State::Pending => {
                // SAFETY: No other threads have access to the waiter.
                let waiter = unsafe { &mut *self.waiter.get() };

                // Check if a notification was received.
                if waiter.notified {
                    // Remove the waiter.
                    unsafe {
                        let waiters = &mut *self.notify.waiters.get();

                        let ptr = NonNull::new_unchecked(self.waiter.get());
                        waiters.remove(ptr);
                    }

                    *self.state_mut() = State::Done;
                    Poll::Ready(())
                } else {
                    // Update the waker if necessary.
                    let update = match &waiter.waker {
                        Some(waker) => !waker.will_wake(cx.waker()),
                        None => true,
                    };

                    if update {
                        waiter.waker = Some(cx.waker().clone());
                    }

                    Poll::Pending
                }
            }
            State::Done => Poll::Ready(()),
        }
    }
}

impl<'a> Drop for Notified<'a> {
    fn drop(&mut self) {
        // Remove the waiter if necessary.
        if self.state == State::Pending {
            // SAFETY: No other threads have access to the waiter list.
            unsafe {
                let waiters = &mut *self.notify.waiters.get();

                waiters.remove(NonNull::new_unchecked(self.waiter.get()));
            }
        }
    }
}

impl<'a> FusedFuture for Notified<'a> {
    #[inline]
    fn is_terminated(&self) -> bool {
        self.state == State::Done
    }
}

#[cfg(test)]
mod tests {
    use std::vec::Vec;
    use std::{rc::Rc, time::Duration};

    use tokio::{sync::mpsc, task::LocalSet};

    use super::Notify;

    #[test]
    fn test_notify_notified() {
        let notify = Notify::new();
        let _: Vec<_> = (0..5).map(|_| notify.notified()).collect();
    }

    #[test]
    fn test_notify_all() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let notify = Rc::new(Notify::new());

        let (tx, mut rx) = mpsc::channel(5);

        let tasks = LocalSet::new();

        for _ in 0..5 {
            let handle = notify.clone();
            let tx = tx.clone();
            tasks.spawn_local(async move {
                handle.notified().await;
                let _ = tx.send(()).await;
            });
        }

        tasks.spawn_local(async move {
            tokio::time::sleep(Duration::new(1, 0)).await;
            notify.notify_all();

            for _ in 0..5 {
                let _ = rx.recv().await;
            }
        });

        rt.block_on(tasks);
    }

    #[test]
    fn test_notify_one_stored() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let notify = Rc::new(Notify::new());
        notify.notify_one();

        let tasks = LocalSet::new();

        tasks.spawn_local(async move {
            tokio::time::sleep(Duration::new(1, 0)).await;
            notify.notified().await;
        });

        rt.block_on(tasks);
    }

    #[test]
    fn test_notify_one() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let notify = Rc::new(Notify::new());

        let tasks = LocalSet::new();

        let handle = notify.clone();
        tasks.spawn_local(async move {
            handle.notified().await;
        });

        tasks.spawn_local(async move {
            tokio::time::sleep(Duration::new(1, 0)).await;
            notify.notify_one();
        });

        rt.block_on(tasks);
    }
}
