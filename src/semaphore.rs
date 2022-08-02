use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Poll, Context};
use core::pin::Pin;
use core::future::Future;

use std::sync::Mutex;

use futures::future::FusedFuture;

use crate::linked_list::LinkedList;
use crate::utils::semaphore::{Waiter, State};
use crate::utils::is_unpin;

#[derive(Debug)]
pub struct Semaphore {
    permits: AtomicUsize,

    waiters: Mutex<LinkedList<Waiter>>,
}

impl Semaphore {
    pub fn new(permits: usize) -> Self {
        Self {
            permits: AtomicUsize::new(permits),
            waiters: Mutex::new(LinkedList::new())
        }
    }

    pub fn avaliable_permits(&self) -> usize {
        self.permits.load(Ordering::SeqCst)
    }

    pub fn accquire(&self) -> Accquire<'_> {
        Accquire {
           semaphore: self,
           waiter: Waiter::new(),
           state: State::Init(1),
        }
    }
}

#[derive(Debug)]
pub struct Accquire<'a> {
    semaphore: &'a Semaphore,
    waiter: Waiter,
    state: State,
}

impl<'a> Accquire<'a> {
    #[inline]
    fn state_mut(self: Pin<&mut Self>) -> &mut State {
        is_unpin::<State>();

        // SAFETY: State is unpin.
        unsafe { &mut self.get_unchecked_mut().state }
    }
}

impl<'a> Future for Accquire<'a> {
    type Output = Permit<'a>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.state {
            State::Init(n) =>  {
                // Check if enough permits exist.
                let res = self.semaphore.permits.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |permits| permits.checked_sub(n));

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

                let res = self.semaphore.permits.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |permits| permits.checked_sub(n));

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
            })
        }
    }
}

impl<'a> Drop for Accquire<'a> {
    fn drop(&mut self) {
        // Remove the waiter if necessary.
        if matches!(self.state, State::Waiting(_)) {
            let mut waiters  = self.semaphore.waiters.lock().unwrap();

            unsafe {
                waiters.remove((&self.waiter).into());
            }
        }
    }
}

impl<'a> FusedFuture for Accquire<'a> {
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

impl<'a> Drop for Permit<'a> {
    fn drop(&mut self) {
        self.semaphore.permits.fetch_add(self.permits, Ordering::SeqCst);
    }
}
