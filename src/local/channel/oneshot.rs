use core::cell::UnsafeCell;
use core::future::Future;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use alloc::rc::Rc;

pub use crate::errors::channel::oneshot::{RecvError, TryRecvError};

const STATE_DEFAULT: u8 = 0;
const STATE_HAS_VALUE: u8 = 0b0000_0001;
const STATE_TX_CLOSED: u8 = 0b0000_0010;
const STATE_RX_CLOSED: u8 = 0b0000_0100;
const STATE_TX_WAKER_INIT: u8 = 0b0100_0000;
const STATE_RX_WAKER_INIT: u8 = 0b1000_0000;

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Rc::new(Inner::new());

    let tx = Sender {
        inner: inner.clone(),
    };
    let rx = Receiver { inner };

    (tx, rx)
}

#[derive(Debug)]
pub struct Sender<T> {
    inner: Rc<Inner<T>>,
}

impl<T> Sender<T> {
    /// Tries the send a value to the assocaited [`Receiver`], returning it the receiver is closed.
    pub fn send(self, value: T) -> Result<(), T> {
        self.inner.send(value)
    }

    /// Returns `true` if the assocaited [`Receiver`] has been closed.
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.inner.is_rx_closed()
    }

    #[inline]
    pub fn closed(&self) -> Closed<'_, T> {
        Closed { tx: self }
    }
}

impl<T> Drop for Sender<T> {
    #[inline]
    fn drop(&mut self) {
        self.inner.close_tx();
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    inner: Rc<Inner<T>>,
}

impl<T> Receiver<T> {
    #[inline]
    pub fn close(&mut self) {
        self.inner.close_rx();
    }
}

impl<T> Future for Receiver<T> {
    type Output = Result<T, RecvError>;

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
pub struct Inner<T> {
    state: UnsafeCell<State>,
    value: UnsafeCell<MaybeUninit<T>>,
    tx_waker: UnsafeCell<MaybeUninit<Waker>>,
    rx_waker: UnsafeCell<MaybeUninit<Waker>>,
}

impl<T> Inner<T> {
    #[inline]
    const fn new() -> Self {
        Self {
            state: UnsafeCell::new(State::new()),
            value: UnsafeCell::new(MaybeUninit::uninit()),
            tx_waker: UnsafeCell::new(MaybeUninit::uninit()),
            rx_waker: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    fn send(&self, value: T) -> Result<(), T> {
        let state = unsafe { self.state() };

        if state.is_rx_closed() {
            Err(value)
        } else {
            state.set_value(true);

            unsafe {
                let cell = &mut *self.value.get();
                cell.write(value);
            }

            Ok(())
        }
    }

    /// Returns a mutable refence the [`State`].
    ///
    /// # Safety
    ///
    /// This method is only safe to call when there are no other references the state.
    #[allow(clippy::mut_from_ref)]
    unsafe fn state(&self) -> &mut State {
        unsafe { &mut *self.state.get() }
    }

    fn is_rx_closed(&self) -> bool {
        unsafe { self.state().is_rx_closed() }
    }

    fn close_tx(&self) {
        let state = unsafe { self.state() };
        state.set_tx_closed(true);

        // Wake the receiver.
        if state.is_rx_waker_init() {
            // SAFETY: RX_WAKER_INIT bit is set, rx_waker is initialized.
            let waker = unsafe {
                let cell = &*self.rx_waker.get();
                cell.assume_init_ref()
            };

            waker.wake_by_ref();
        }
    }

    fn close_rx(&self) {
        let state = unsafe { self.state() };
        state.set_rx_closed(true);

        // Wake the sender.
        if state.is_tx_waker_init() {
            // SAFETY: TX_WAKER_INIT bit is set, tx_waker is initialized.
            let waker = unsafe {
                let cell = &*self.tx_waker.get();
                cell.assume_init_ref()
            };

            waker.wake_by_ref();
        }
    }

    fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Result<T, RecvError>> {
        let state = unsafe { self.state() };

        match state {
            state if state.has_value() => {
                state.set_value(false);

                // SAFETY: The HAS_VALUE bit is set.
                unsafe { Poll::Ready(Ok(self.read_value())) }
            }
            state if state.is_tx_closed() => Poll::Ready(Err(RecvError::new())),
            _ => {
                let cell = unsafe { &mut *self.rx_waker.get() };

                // Update the waker if necessary.
                let update = if state.is_rx_waker_init() {
                    // SAFETY: The RX_WAKER_INIT bit is set, the field is initialized.
                    let waker = unsafe { cell.assume_init_mut() };

                    if !waker.will_wake(cx.waker()) {
                        // SAFETY: The field is initialized.
                        unsafe {
                            cell.assume_init_drop();
                        }

                        true
                    } else {
                        false
                    }
                } else {
                    state.set_rx_waker_init(true);
                    true
                };

                if update {
                    cell.write(cx.waker().clone());
                }

                Poll::Pending
            }
        }
    }

    fn poll_closed(&self, cx: &mut Context<'_>) -> Poll<()> {
        let state = unsafe { self.state() };

        match state {
            state if state.is_rx_closed() => Poll::Ready(()),
            _ => {
                let cell = unsafe { &mut *self.tx_waker.get() };

                let update = if state.is_tx_waker_init() {
                    let waker = unsafe { cell.assume_init_mut() };

                    if !waker.will_wake(cx.waker()) {
                        unsafe {
                            cell.assume_init_drop();
                        }

                        true
                    } else {
                        false
                    }
                } else {
                    state.set_tx_waker_init(true);
                    true
                };

                if update {
                    cell.write(cx.waker().clone());
                }

                Poll::Pending
            }
        }
    }

    unsafe fn read_value(&self) -> T {
        unsafe {
            let value = &*self.value.get();
            value.assume_init_read()
        }
    }
}

impl<T> Drop for Inner<T> {
    fn drop(&mut self) {
        let state = self.state.get_mut();

        // Manually drop the contained value.
        if state.has_value() {
            // SAFETY: The `STATE_HAS_VALUE` bit indicates that the value
            // is initialized.
            unsafe {
                self.value.get_mut().assume_init_drop();
            }
        }

        // Drop the wakers if initialized.
        if state.is_tx_waker_init() {
            // SAFETY: The `STATE_TX_WAKER_INIT` bit indicates that the waker
            // is initialized.
            unsafe {
                self.tx_waker.get_mut().assume_init_drop();
            }
        }

        if state.is_rx_waker_init() {
            // SAFETY: The `STATE_RX_WAKER_INIt` bit indicates that the waker
            // is initialized.
            unsafe {
                self.rx_waker.get_mut().assume_init_drop();
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct State(u8);

impl State {
    #[inline]
    const fn new() -> Self {
        Self(STATE_DEFAULT)
    }

    #[inline]
    const fn has_value(&self) -> bool {
        self.0 & STATE_HAS_VALUE != 0
    }

    fn set_value(&mut self, value: bool) {
        if value {
            self.0 |= STATE_HAS_VALUE;
        } else {
            self.0 &= !STATE_HAS_VALUE;
        }
    }

    fn is_tx_closed(&self) -> bool {
        self.0 & STATE_TX_CLOSED != 0
    }

    fn set_tx_closed(&mut self, value: bool) {
        if value {
            self.0 |= STATE_TX_CLOSED;
        } else {
            self.0 &= !STATE_TX_CLOSED;
        }
    }

    fn is_rx_closed(&self) -> bool {
        self.0 & STATE_RX_CLOSED != 0
    }

    fn set_rx_closed(&mut self, value: bool) {
        if value {
            self.0 |= STATE_RX_CLOSED;
        } else {
            self.0 &= !STATE_RX_CLOSED;
        }
    }

    fn is_tx_waker_init(&self) -> bool {
        self.0 & STATE_TX_WAKER_INIT != 0
    }

    fn set_tx_waker_init(&mut self, value: bool) {
        if value {
            self.0 |= STATE_TX_WAKER_INIT;
        } else {
            self.0 &= !STATE_TX_WAKER_INIT;
        }
    }

    fn is_rx_waker_init(&self) -> bool {
        self.0 & STATE_RX_WAKER_INIT != 0
    }

    fn set_rx_waker_init(&mut self, value: bool) {
        if value {
            self.0 |= STATE_RX_WAKER_INIT;
        } else {
            self.0 &= !STATE_RX_WAKER_INIT;
        }
    }
}

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
mod tests {}
