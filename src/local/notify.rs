use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::marker::PhantomData;

use crate::is_unpin;
use crate::linked_list::LinkedList;
use crate::notify::{State, Waiter};

#[derive(Debug)]
pub struct Notify {
    state: UnsafeCell<usize>,
    waiters: UnsafeCell<LinkedList<Waiter>>,

    _marker: PhantomData<*const ()>,
}

impl Notify {
    #[inline]
    pub fn new() -> Self {
        Self {
            state: UnsafeCell::new(0),
            waiters: UnsafeCell::new(LinkedList::new()),
            _marker: PhantomData,
        }
    }

    pub fn notify_all(&self) {
        unsafe {
            *(&mut *self.state.get()) = 0;
            let waiters = &mut *self.waiters.get();

            for waiter in waiters.iter_mut() {
                let waiter = waiter.get();

                waiter.notified = true;

                if let Some(waker) = &waiter.waker {
                    waker.wake_by_ref();
                }
            }
        }
    }

    pub fn notify_one(&self) {
        let waiters = unsafe { &mut *self.waiters.get() };

        match waiters.front() {
            Some(waiter) => {
                let waiter = unsafe { waiter.get() };

                waiter.notified = true;
                if let Some(waker) = &waiter.waker {
                    waker.wake_by_ref();
                }
            }
            None => unsafe { *(&mut *self.state.get()) = 1 },
        }
    }

    pub fn notifed(&self) -> Notified<'_> {
        Notified {
            notify: self,
            state: State::Init,
            waiter: Waiter::new(),
        }
    }
}

#[derive(Debug)]
pub struct Notified<'a> {
    notify: &'a Notify,
    state: State,
    waiter: Waiter,
}

impl<'a> Notified<'a> {
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
                let state = unsafe { &mut *self.notify.state.get() };
                if *state == 1 {
                    *state = 0;
                    *self.state_mut() = State::Done;
                    return Poll::Ready(());
                }

                drop(state);

                // Register new waiter.
                unsafe {
                    self.waiter.get().waker = Some(cx.waker().clone());

                    let waiters = &mut *self.notify.waiters.get();

                    waiters.push_back((&self.waiter).into());
                    drop(waiters);
                };

                *self.state_mut() = State::Pending;
                Poll::Pending
            }
            State::Pending => {
                let waiter = unsafe { self.waiter.get() };

                if waiter.notified {
                    // Remove the waiter.
                    unsafe {
                        let waiters = &mut *self.notify.waiters.get();
                        waiters.remove((&self.waiter).into());
                    }

                    *self.state_mut() = State::Done;
                    Poll::Ready(())
                } else {
                    let update = match &waiter.waker {
                        Some(waker) => !waker.will_wake(&cx.waker()),
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

#[cfg(test)]
mod tests {
    use std::{rc::Rc, time::Duration};

    use tokio::{sync::mpsc, task::LocalSet};

    use super::Notify;

    #[test]
    fn test_notify_notified() {
        let notify = Notify::new();
        let _: Vec<_> = (0..5).map(|_| notify.notifed()).collect();
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
                handle.notifed().await;
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
            notify.notifed().await;
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
            handle.notifed().await;
        });

        tasks.spawn_local(async move {
            tokio::time::sleep(Duration::new(1, 0)).await;
            notify.notify_one();
        });

        rt.block_on(tasks);
    }
}
