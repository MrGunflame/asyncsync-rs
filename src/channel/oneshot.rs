//! A one-shot channel for sending a single value between tasks.
//!
//! Use the [`channel`] function to create a new [`Sender`]-[`Receiver`] pair.
//!
//! The [`Sender`] and [`Receiver`] halves can be sent to different threads. If sharing between
//! threads is not required, [`oneshot`] is a better option.
//!
//! # Examples
//!
//! ```
//! use asyncsync::channel::oneshot;
//!
//! #[tokio::main]
//! async fn main() {
//!     let (tx, rx) = oneshot::channel();
//!
//!     tokio::task::spawn(async move {
//!         tx.send(42).unwrap();
//!     });
//!
//!     let value = rx.await.unwrap();
//!     assert_eq!(value, 42);
//! }
//! ```
//!
//! [`oneshot`]: crate::local::channel::oneshot

use core::cell::UnsafeCell;
use core::future::Future;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::sync::atomic::{AtomicU8, Ordering};
use core::task::{Context, Poll, Waker};

use std::sync::Arc;

use parking_lot::Mutex;

pub use crate::errors::channel::oneshot::{RecvError, TryRecvError};

/// This bit is set when the channel contains a value. Only then is it safe to
/// read it.
const STATE_HAS_VALUE: u8 = 0b0000_0001;
const STATE_TX_CLOSED: u8 = 0b0000_0010;
const STATE_RX_CLOSED: u8 = 0b0000_0100;

/// Creates a new one-shot channel.
///
/// # Examples
///
/// ```
/// use asyncsync::channel::oneshot;
///
/// #[tokio::main]
/// async fn main() {
///     let (tx, rx) = oneshot::channel();
///
///     tokio::task::spawn(async move {
///         tx.send(42).unwrap();
///     });
///
///     assert_eq!(rx.await.unwrap(), 42);
/// }
/// ```
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Arc::new(Inner::new());

    (
        Sender {
            inner: inner.clone(),
        },
        Receiver { inner },
    )
}

/// The sending half of a oneshot channel.
///
/// Use [`channel`] to create a new `Sender`-[`Receiver`] pair.
#[derive(Debug)]
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Sender<T> {
    /// Returns `true` if the associated [`Receiver`] is closed.
    ///
    /// # Examples
    ///
    /// ```
    /// use asyncsync::channel::oneshot;
    ///
    /// let (tx, mut rx) = oneshot::channel::<()>();
    /// assert!(!tx.is_closed());
    ///
    /// rx.close();
    /// assert!(tx.is_closed());
    /// ```
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.inner.is_rx_closed()
    }

    /// Waits for the associated [`Receiver`] to close.
    ///
    /// # Examples
    ///
    /// ```
    /// use asyncsync::channel::oneshot;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx, mut rx) = oneshot::channel::<()>();
    ///     
    ///     tokio::task::spawn(async move {
    ///         rx.close();
    ///     });
    ///
    ///     tx.closed().await;
    /// }
    /// ```
    #[inline]
    pub fn closed(&self) -> Closed<'_, T> {
        Closed { tx: self }
    }

    /// Tries to send a `value` to the associated [`Receiver`], returning it back if the receiver
    /// is closed.
    ///
    /// # Examples
    ///
    /// ```
    /// use asyncsync::channel::oneshot;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx, rx) = oneshot::channel();
    ///
    ///     tokio::task::spawn(async move {
    ///         tx.send(42).unwrap();
    ///     });
    ///
    ///     assert_eq!(rx.await.unwrap(), 42);
    /// }
    /// ```
    ///
    /// ```
    /// use asyncsync::channel::oneshot;
    ///
    /// let (tx, rx) = oneshot::channel();
    /// drop(rx);
    ///
    /// // The receiver was already dropped.
    /// assert_eq!(tx.send(42).unwrap_err(), 42);
    /// ```
    #[inline]
    pub fn send(self, value: T) -> Result<(), T> {
        self.inner.send(value)
    }
}

impl<T> Drop for Sender<T> {
    #[inline]
    fn drop(&mut self) {
        self.inner.close_tx();
    }
}

/// The receiving half of a oneshot channel.
///
/// Use [`channel`] to create a new [`Sender`]-`Receiver` pair.
#[derive(Debug)]
pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Receiver<T> {
    /// Closes the channel, preventing the associated [`Sender`] from sending any more messages.
    ///
    /// Note that all already sent messages will stay in the channel. Polling this `Receiver` or
    /// calling `try_recv` will take the existing value from the channel first.
    ///
    /// # Examples
    ///
    /// ```
    /// use asyncsync::channel::oneshot;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx, mut rx) = oneshot::channel::<()>();
    ///
    ///     tokio::task::spawn(async move {
    ///         rx.close();
    ///     });
    ///
    ///     tx.closed().await;
    /// }
    /// ```
    #[inline]
    pub fn close(&mut self) {
        self.inner.close_rx();
    }

    /// Tries the read a value from the channel. If the channel contains a value, it is returned.
    ///
    /// # Errors
    ///
    /// Returns a [`TryRecvError::Empty`] when the channel contains no value currently.
    /// Returns a [`TryRecvError::Closed`] when the channel contains no value and the associated
    /// [`Sender`] was dropped. Any future calls to `try_recv` will fail.
    ///
    /// # Examples
    ///
    /// ```
    /// use asyncsync::channel::oneshot::{self, TryRecvError};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx, mut rx) = oneshot::channel();
    ///
    ///     assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
    ///
    ///     tx.send(123).unwrap();
    ///     assert_eq!(rx.try_recv().unwrap(), 123);
    ///
    ///     assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Closed);
    /// }
    /// ```
    #[inline]
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.inner.try_recv()
    }
}

impl<T> Future for Receiver<T> {
    type Output = Result<T, RecvError>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.poll_recv(cx)
    }
}

impl<T> Drop for Receiver<T> {
    #[inline]
    fn drop(&mut self) {
        self.inner.close_rx();
    }
}

#[derive(Debug)]
struct Inner<T> {
    // 0 = open, 1 = closed
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
    // The waker of the Sender. Used to notify a close.
    tx_waker: Mutex<Option<Waker>>,
    // The waker of the Receiver. Used to notify a sent message.
    rx_waker: Mutex<Option<Waker>>,
}

impl<T> Inner<T> {
    #[inline]
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(0),
            value: UnsafeCell::new(MaybeUninit::uninit()),
            tx_waker: Mutex::new(None),
            rx_waker: Mutex::new(None),
        }
    }

    fn send(&self, value: T) -> Result<(), T> {
        let res =
            self.state
                .compare_exchange(0, STATE_HAS_VALUE, Ordering::SeqCst, Ordering::SeqCst);

        match res {
            Ok(_) => {
                unsafe {
                    let cell = &mut *self.value.get();
                    cell.write(value);
                }

                // Wake the receiver.
                self.wake_rx();
                Ok(())
            }
            Err(_) => Err(value),
        }
    }

    #[inline]
    fn state(&self) -> u8 {
        self.state.load(Ordering::SeqCst)
    }

    /// Closes the [`Sender`] of the channel.
    fn close_tx(&self) {
        self.state.fetch_or(STATE_TX_CLOSED, Ordering::SeqCst);

        // Wake the sender.
        self.wake_rx();
    }

    /// Closes the [`Receiver`] of the channel.
    fn close_rx(&self) {
        self.state.fetch_or(STATE_RX_CLOSED, Ordering::SeqCst);

        self.wake_tx();
    }

    /// Returns `true` if the [`Receiver`] of the channel is closed.
    #[inline]
    fn is_rx_closed(&self) -> bool {
        self.state.load(Ordering::SeqCst) & STATE_RX_CLOSED != 0
    }

    fn wake_tx(&self) {
        let waker = self.tx_waker.lock();
        if let Some(waker) = waker.as_ref() {
            waker.wake_by_ref();
        }
    }

    fn wake_rx(&self) {
        let waker = self.rx_waker.lock();
        if let Some(waker) = waker.as_ref() {
            waker.wake_by_ref();
        }
    }

    fn try_recv(&self) -> Result<T, TryRecvError> {
        let state = self.state();

        match state {
            state if state & STATE_HAS_VALUE != 0 => {
                self.state.store(state & !STATE_HAS_VALUE, Ordering::SeqCst);

                // SAFETY: HAS_VALUE bit is set.
                unsafe { Ok(self.read_value()) }
            }
            state if state & STATE_TX_CLOSED != 0 => Err(TryRecvError::Closed),
            _ => Err(TryRecvError::Empty),
        }
    }

    fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Result<T, RecvError>> {
        let state = self.state.load(Ordering::SeqCst);

        match state {
            state if state & STATE_HAS_VALUE != 0 => {
                // Take the value.
                self.state.store(state & !STATE_HAS_VALUE, Ordering::SeqCst);

                // SAFETY: HAS_VALUE bit is set.
                unsafe { Poll::Ready(Ok(self.read_value())) }
            }
            state if state & STATE_TX_CLOSED != 0 => Poll::Ready(Err(RecvError::new())),
            // Value not yet written.
            _ => {
                // Update waker if necessary.
                let mut waker = self.rx_waker.lock();

                let should_update = match waker.as_ref() {
                    Some(waker) => !waker.will_wake(cx.waker()),
                    None => true,
                };

                if should_update {
                    *waker = Some(cx.waker().clone());
                }

                Poll::Pending
            }
        }
    }

    fn poll_closed(&self, cx: &mut Context<'_>) -> Poll<()> {
        let state = self.state.load(Ordering::SeqCst);

        if state & STATE_TX_CLOSED == 0 {
            Poll::Ready(())
        } else {
            let mut waker = self.tx_waker.lock();

            let should_update = match waker.as_ref() {
                Some(waker) => !waker.will_wake(cx.waker()),
                None => true,
            };

            if should_update {
                *waker = Some(cx.waker().clone());
            }

            Poll::Pending
        }
    }

    /// Take the contained value from the channel buffer.
    ///
    /// # Safety
    ///
    /// This method is only safe to call when the value has been written successfully and the state
    /// has set the STATE_WRITTEN bit. It is only safe to call ONCE.
    #[inline]
    unsafe fn read_value(&self) -> T {
        // SAFETY: The caller must guarantee that `self.value` has been initialized
        // and does not have any active references.
        unsafe {
            let val = &mut *self.value.get();
            val.assume_init_read()
        }
    }
}

impl<T> Drop for Inner<T> {
    fn drop(&mut self) {
        let state = *self.state.get_mut();

        // Drop the value when it wasn't read.
        if state & STATE_HAS_VALUE != 0 {
            // SAFETY: The `STATE_HAS_VALUE` bit indicates that the value
            // is initialized.
            unsafe {
                self.value.get_mut().assume_init_drop();
            }
        }
    }
}

unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Sync> Sync for Inner<T> {}

/// A future waiting for the [`Receiver`] of a channel to be closed.
///
/// `Closed` is created by [`closed`].
///
/// [`closed`]: Sender::closed
#[derive(Debug)]
pub struct Closed<'a, T> {
    tx: &'a Sender<T>,
}

impl<'a, T> Future for Closed<'a, T> {
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.tx.inner.poll_closed(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::{channel, TryRecvError};

    #[tokio::test]
    async fn test_channel() {
        let (tx, rx) = channel();

        tokio::task::spawn(async move {
            tx.send(()).unwrap();
        });

        rx.await.unwrap();
    }

    #[tokio::test]
    async fn test_channel_drop_tx() {
        let (tx, rx) = channel::<()>();

        tokio::task::spawn(async move {
            drop(tx);
        });

        rx.await.unwrap_err();
    }

    #[test]
    fn test_channel_close() {
        let (tx, mut rx) = channel::<()>();

        rx.close();
        tx.send(()).unwrap_err();
    }

    #[tokio::test]
    async fn test_channel_closed() {
        let (tx, mut rx) = channel::<()>();

        tokio::task::spawn(async move {
            rx.close();
        });

        tx.closed().await;
    }

    #[test]
    fn test_channel_is_closed() {
        let (tx, mut rx) = channel::<()>();

        assert!(!tx.is_closed());
        rx.close();
        assert!(tx.is_closed());
    }

    #[test]
    fn test_channel_try_recv() {
        let (tx, mut rx) = channel::<()>();

        assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);

        tx.send(()).unwrap();
        rx.try_recv().unwrap();

        assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Closed);

        let (tx, mut rx) = channel::<()>();
        assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);

        drop(tx);
        assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Closed);
    }
}
