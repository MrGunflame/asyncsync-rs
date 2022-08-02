use core::future::Future;
use core::mem;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll};

use std::sync::Mutex;

use futures::future::FusedFuture;

use crate::linked_list::LinkedList;
use crate::utils::is_unpin;
use crate::utils::semaphore::{State, Waiter};

#[derive(Debug)]
pub struct Semaphore {
    permits: AtomicUsize,

    waiters: Mutex<LinkedList<Waiter>>,
}

impl Semaphore {
    #[inline]
    pub fn new(permits: usize) -> Self {
        Self {
            permits: AtomicUsize::new(permits),
            waiters: Mutex::new(LinkedList::new()),
        }
    }

    #[inline]
    pub fn avaliable_permits(&self) -> usize {
        self.permits.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn add_permits(&self, n: usize) {
        self.permits.fetch_add(n, Ordering::SeqCst);
    }

    #[inline]
    pub fn acquire(&self) -> Acquire<'_> {
        self.acquire_many(1)
    }

    #[inline]
    pub fn acquire_many(&self, n: usize) -> Acquire<'_> {
        Acquire {
            semaphore: self,
            waiter: Waiter::new(),
            state: State::Init(n),
        }
    }

    #[inline]
    pub fn try_acquire(&self) -> Option<Permit<'_>> {
        self.try_acquire_many(1)
    }

    #[inline]
    pub fn try_acquire_many(&self, n: usize) -> Option<Permit<'_>> {
        match self
            .permits
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |permits| {
                permits.checked_sub(n)
            }) {
            Ok(_) => Some(Permit {
                semaphore: self,
                permits: n,
            }),
            Err(_) => None,
        }
    }
}

unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}

#[derive(Debug)]
pub struct Acquire<'a> {
    semaphore: &'a Semaphore,
    waiter: Waiter,
    state: State,
}

impl<'a> Acquire<'a> {
    #[inline]
    fn state_mut(self: Pin<&mut Self>) -> &mut State {
        is_unpin::<State>();

        // SAFETY: State is unpin.
        unsafe { &mut self.get_unchecked_mut().state }
    }
}

impl<'a> Future for Acquire<'a> {
    type Output = Permit<'a>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.state {
            State::Init(n) => {
                // Check if enough permits exist.
                let res = self.semaphore.permits.fetch_update(
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                    |permits| permits.checked_sub(n),
                );

                if res.is_ok() {
                    return Poll::Ready(Permit {
                        semaphore: self.semaphore,
                        permits: n,
                    });
                }

                // Register new waiter
                let mut waiters = self.semaphore.waiters.lock().unwrap();

                unsafe {
                    self.waiter.get().waker = Some(cx.waker().clone());
                    waiters.push_back((&self.waiter).into());
                }

                drop(waiters);

                *self.state_mut() = State::Waiting(n);
                Poll::Pending
            }
            State::Waiting(n) => {
                let mut waiters = self.semaphore.waiters.lock().unwrap();

                let waiter = unsafe { self.waiter.get() };

                let res = self.semaphore.permits.fetch_update(
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                    |permits| permits.checked_sub(n),
                );

                if res.is_ok() {
                    // Remove the waiter.
                    unsafe {
                        waiters.remove((&self.waiter).into());
                    }

                    *self.as_mut().state_mut() = State::Done;
                    Poll::Ready(Permit {
                        semaphore: self.semaphore,
                        permits: n,
                    })
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
            State::Done => Poll::Ready(Permit {
                semaphore: self.semaphore,
                permits: 0,
            }),
        }
    }
}

unsafe impl<'a> Send for Acquire<'a> {}
unsafe impl<'a> Sync for Acquire<'a> {}

impl<'a> Drop for Acquire<'a> {
    fn drop(&mut self) {
        // Remove the waiter if necessary.
        if matches!(self.state, State::Waiting(_)) {
            let mut waiters = self.semaphore.waiters.lock().unwrap();

            unsafe {
                waiters.remove((&self.waiter).into());
            }
        }
    }
}

impl<'a> FusedFuture for Acquire<'a> {
    #[inline]
    fn is_terminated(&self) -> bool {
        self.state == State::Done
    }
}

#[derive(Debug)]
pub struct Permit<'a> {
    semaphore: &'a Semaphore,
    permits: usize,
}

impl<'a> Permit<'a> {
    pub fn forget(self) {
        mem::forget(self);
    }
}

impl<'a> Drop for Permit<'a> {
    #[inline]
    fn drop(&mut self) {
        self.semaphore.add_permits(self.permits);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;
    use std::vec::Vec;

    use tokio::sync::mpsc;

    use super::Semaphore;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_semaphore() {
        let semaphore = Arc::new(Semaphore::new(5));

        let (tx, mut rx) = mpsc::channel(10);

        for _ in 0..10 {
            let semaphore = semaphore.clone();
            let tx = tx.clone();

            tokio::task::spawn(async move {
                semaphore.acquire().await;
                tokio::time::sleep(Duration::new(1, 0)).await;
                let _ = tx.send(()).await;
            });
        }

        for _ in 0..10 {
            let _ = rx.recv().await;
        }
    }

    #[test]
    fn test_semaphore_acquire() {
        let semaphore = Semaphore::new(0);
        let _: Vec<_> = (0..5).map(|_| semaphore.acquire()).collect();
    }
}
