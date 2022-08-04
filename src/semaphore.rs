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

/// A semaphore with asynchronous permit acquisition.
///
/// This `Semaphore` is implemented fairly, meaning that permits are returned in the order
/// they were requested. This includes `acquire_many`. A call to `acquire_many` will block
/// the semaphore until enough permits are avaliable for it, even if that blocks calls to
/// `acquire`.
///
/// # Examples
///
/// ```
/// use asyncsync::Semaphore;
///
/// #[tokio::main]
/// async fn main() {
///     let semaphore = Semaphore::new(1);
///
///     let permit = semaphore.acquire().await;
///     assert_eq!(semaphore.available_permits(), 0);
///
///     assert!(semaphore.try_acquire().is_none());
/// }
/// ```
#[derive(Debug)]
pub struct Semaphore {
    permits: AtomicUsize,

    waiters: Mutex<LinkedList<Waiter>>,
}

impl Semaphore {
    /// Creates a new `Semaphore` with the staring number of permits.
    #[inline]
    pub fn new(permits: usize) -> Self {
        Self {
            permits: AtomicUsize::new(permits),
            waiters: Mutex::new(LinkedList::new()),
        }
    }

    /// Returns the number of permits currently available.
    ///
    /// # Examples
    ///
    /// ```
    /// # use asyncsync::semaphore::Semaphore;
    /// #
    /// let semaphore = Semaphore::new(3);
    /// assert_eq!(semaphore.available_permits(), 3);
    ///
    /// semaphore.try_acquire().unwrap().forget();
    /// assert_eq!(semaphore.available_permits(), 2);
    /// ```
    #[inline]
    pub fn available_permits(&self) -> usize {
        self.permits.load(Ordering::SeqCst)
    }

    /// Adds `n` new permits to the `Semaphore`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use asyncsync::semaphore::Semaphore;
    /// #
    /// let semaphore = Semaphore::new(3);
    /// assert_eq!(semaphore.available_permits(), 3);
    ///
    /// semaphore.add_permits(3);
    /// assert_eq!(semaphore.available_permits(), 6);
    /// ```
    pub fn add_permits(&self, n: usize) {
        let mut permits = self.permits.fetch_add(n, Ordering::SeqCst) + n;

        let mut waiters = self.waiters.lock().unwrap();
        let iter = waiters.iter_mut();
        for waiter in iter {
            let waiter = unsafe { waiter.get() };

            if permits < waiter.permits {
                break;
            }

            if let Some(waker) = &waiter.waker {
                waker.wake_by_ref();
            }

            permits -= waiter.permits;
        }
    }

    /// Acquire a single permit.
    ///
    /// # Examples
    ///
    /// ```
    /// # use asyncsync::Semaphore;
    /// #
    /// #[tokio::main]
    /// async fn main() {
    ///     let semaphore = Semaphore::new(1);
    ///
    ///     let permit = semaphore.acquire().await;
    /// }
    /// ```
    #[inline]
    pub fn acquire(&self) -> Acquire<'_> {
        self.acquire_many(1)
    }

    /// Acquire multiple permits.
    ///
    /// # Examples
    ///
    /// ```
    /// # use asyncsync::Semaphore;
    /// #
    /// #[tokio::main]
    /// async fn main() {
    ///     let semaphore = Semaphore::new(5);
    ///
    ///     let permit = semaphore.acquire_many(5).await;
    /// }
    /// ```
    #[inline]
    pub fn acquire_many(&self, n: usize) -> Acquire<'_> {
        Acquire {
            semaphore: self,
            waiter: Waiter::new(n),
            state: State::Init(n),
        }
    }

    /// Tries to acquire a single permit. Returns `None` if no permit is avaliable.
    ///
    /// # Examples
    ///
    /// ```
    /// # use asyncsync::semaphore::Semaphore;
    /// #
    /// let semaphore = Semaphore::new(1);
    ///
    /// let permit = semaphore.try_acquire().unwrap();
    ///
    /// assert!(semaphore.try_acquire().is_none());
    /// ```
    #[inline]
    pub fn try_acquire(&self) -> Option<Permit<'_>> {
        self.try_acquire_many(1)
    }

    /// Tries to acquire `n` permits. Returns `None` if not enough permits are avaliable.
    ///
    /// # Examples
    ///
    /// ```
    /// # use asyncsync::semaphore::Semaphore;
    /// #
    /// let semaphore = Semaphore::new(5);
    ///
    /// let permit = semaphore.try_acquire_many(5).unwrap();
    ///
    /// assert!(semaphore.try_acquire_many(1).is_none());
    /// ```
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

/// A future waiting for permits to become avaliable.
///
/// `Acquire` is returned by [`Semaphore::acquire`] and [`Semaphore::acquire_many`].
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
                let mut waiters = self.semaphore.waiters.lock().unwrap();

                // Only check the permits at the start when the waitlist is empty.
                // This is required to preserve the fairness.
                if waiters.is_empty() {
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
                }

                // Register new waiter
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

/// An acquired permit from a [`Semaphore`].
///
/// The permit is returned to the owning semaphore when dropped unless `forget` is called.
///
/// [`forget`]: Self::forget
#[derive(Debug)]
pub struct Permit<'a> {
    semaphore: &'a Semaphore,
    permits: usize,
}

impl<'a> Permit<'a> {
    /// Drops the permit without returning it back to the owning [`Semaphore`].
    #[inline]
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

    use tokio::sync::{mpsc, oneshot};

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_semaphore_wake() {
        let semaphore = Arc::new(Semaphore::new(0));

        let (tx, rx) = oneshot::channel();
        let handle = semaphore.clone();
        tokio::task::spawn(async move {
            handle.acquire().await;
            let _ = tx.send(());
        });

        tokio::time::sleep(Duration::new(5, 0)).await;
        semaphore.add_permits(1);
        let _ = rx.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_semaphore_wake_many() {
        let semaphore = Arc::new(Semaphore::new(0));

        let (tx, mut rx) = mpsc::channel(5);
        for _ in 0..5 {
            let semaphore = semaphore.clone();
            let tx = tx.clone();

            tokio::task::spawn(async move {
                semaphore.acquire().await;
                let _ = tx.send(()).await;
            });
        }

        tokio::time::sleep(Duration::new(5, 0)).await;
        semaphore.add_permits(5);

        for _ in 0..5 {
            let _ = rx.recv().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_semaphore_acquire() {
        let semaphore = Arc::new(Semaphore::new(5));

        let (tx, mut rx) = mpsc::channel(10);

        for _ in 0..10 {
            let semaphore = semaphore.clone();
            let tx = tx.clone();

            tokio::task::spawn(async move {
                semaphore.acquire().await;
                tokio::time::sleep(Duration::new(5, 0)).await;
                let _ = tx.send(()).await;
            });
        }

        for _ in 0..10 {
            let _ = rx.recv().await;
        }
    }

    #[test]
    fn test_semaphore_try_acquire() {
        let semaphore = Semaphore::new(5);

        assert!(semaphore.try_acquire_many(6).is_none());

        let permit = semaphore.try_acquire_many(5).unwrap();
        assert_eq!(semaphore.available_permits(), 0);

        assert!(semaphore.try_acquire().is_none());

        drop(permit);
        let _permit = semaphore.try_acquire().unwrap();
    }

    #[tokio::test]
    async fn test_semaphore_accquire_accumulate() {
        let semaphore = Arc::new(Semaphore::new(0));

        let (tx, rx) = oneshot::channel();

        let handle = semaphore.clone();
        tokio::task::spawn(async move {
            handle.acquire_many(5).await;
            let _ = tx.send(());
        });

        for _ in 0..5 {
            semaphore.add_permits(1);
        }

        let _ = rx.await;
    }
}
