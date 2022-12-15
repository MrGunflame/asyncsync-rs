use core::cell::UnsafeCell;
use core::future::Future;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::task::{ready, Context, Poll, Waker};

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use parking_lot::Mutex;

use crate::errors::channel::mpsc::{TryRecvError, TrySendError};
use crate::linked_list::LinkedList;
use crate::utils::channel::mpsc::Waiter;

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    unimplemented!()
}

#[derive(Debug)]
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Sender<T> {
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    pub async fn close(&self) {
        self.inner.close();
    }

    pub fn try_send(&mut self, value: T) -> Result<(), TrySendError<T>> {
        self.inner.try_send(value)
    }

    pub async fn send(&mut self, value: T) -> Result<(), T> {
        unimplemented!()
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Receiver<T> {
    pub fn recv(&mut self) -> Recv<'_, T> {
        Recv { inner: &self.inner }
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.inner.try_recv()
    }

    pub fn close(&mut self) {
        unimplemented!()
    }
}

#[derive(Debug)]
struct Inner<T> {
    buffer: Box<[Mutex<MaybeUninit<T>>]>,
    next_read: AtomicUsize,
    next_write: AtomicUsize,
    rx_waker: Mutex<Option<Waker>>,
    tx_wakers: Mutex<LinkedList<Waiter>>,
    closed: AtomicBool,
}

impl<T> Inner<T> {
    fn new(size: usize) -> Self {
        let mut buf = Vec::with_capacity(size);
        for i in 0..size {
            buf[i] = Mutex::new(MaybeUninit::uninit());
        }

        Self {
            buffer: buf.into_boxed_slice(),
            next_read: AtomicUsize::new(0),
            next_write: AtomicUsize::new(0),
            rx_waker: Mutex::new(None),
            tx_wakers: Mutex::new(LinkedList::new()),
            closed: AtomicBool::new(false),
        }
    }

    fn try_recv(&self) -> Result<T, TryRecvError> {
        let read = self.next_read.load(Ordering::SeqCst);
        let write = self.next_write.load(Ordering::SeqCst);

        if read < write {
            let pos = read % self.buffer.len();

            let elem = self.buffer[pos].lock();

            self.next_read.store(read + 1, Ordering::SeqCst);

            Ok(unsafe { elem.assume_init_read() })
        } else {
            if self.closed.load(Ordering::Relaxed) {
                Err(TryRecvError::Closed)
            } else {
                Err(TryRecvError::Empty)
            }
        }
    }

    fn try_send(&self, item: T) -> Result<(), TrySendError<T>> {
        let mut pos = 0;
        loop {
            let read = self.next_read.load(Ordering::SeqCst);
            let write = self.next_write.load(Ordering::SeqCst);

            if self.buffer.len() - (write - read) > 0 {
                if self
                    .next_write
                    .compare_exchange_weak(write, write + 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    pos = write % self.buffer.len();
                    break;
                }
            } else {
                return Err(TrySendError::Full(item));
            }
        }

        self.buffer[pos].lock().write(item);
        self.wake_rx();
        Ok(())
    }

    fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        if let Ok(item) = self.try_recv() {
            return Poll::Ready(Some(item));
        }

        let mut rx_waker = self.rx_waker.lock();

        let should_update = match rx_waker.as_ref() {
            Some(waker) => waker.will_wake(waker),
            None => true,
        };

        if should_update {
            *rx_waker = Some(cx.waker().clone());
        }

        Poll::Pending
    }

    fn poll_send(&self, cx: &mut Context<'_>, value: T) -> Poll<Result<(), T>> {
        match self.try_send(value) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(TrySendError::Closed(val)) => Poll::Ready(Err(val)),
            _ => (),
        }
    }

    fn wake_rx(&self) {
        let waker = self.rx_waker.lock();
        if let Some(waker) = &*waker {
            waker.wake_by_ref();
        }
    }

    fn wake_tx(&self) {
        let mut waiters = self.tx_wakers.lock();

        for waiter in waiters.iter_mut() {
            if let Some(waker) = &waiter.waker {
                waker.wake_by_ref();
            }
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.wake_tx();
    }
}

#[derive(Debug)]
pub struct Recv<'a, T> {
    inner: &'a Inner<T>,
}

impl<'a, T> Future for Recv<'a, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.poll_recv(cx)
    }
}

#[derive(Debug)]
pub struct Send<'a, T> {
    inner: &'a Inner<T>,
    /// The value to be sent.
    // FIXME: This might be better as a MaybeUninit.
    value: Option<T>,
    state: State,
    waiter: UnsafeCell<Waiter>,
}

impl<'a, T> Future for Send<'a, T> {
    type Output = Result<(), T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.state {
            State::Init => match self.inner.poll_send(cx, self.value.take()) {
                Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
                Poll::Pending => Poll::Pending,
            },
            State::Pending => {}
            State::Done => Poll::Ready(Ok(())),
        }
    }
}

impl<'a, T> Drop for Send<'a, T> {
    fn drop(&mut self) {
        if self.state == State::Pending {
            let mut waiters = self.inner.tx_wakers.lock();

            unsafe {
                let ptr = NonNull::new_unchecked(self.waiter.get());
                waiters.remove(ptr);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Init,
    Pending,
    Done,
}
