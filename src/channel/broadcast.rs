use core::{cell::UnsafeCell, sync::atomic::AtomicUsize};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use parking_lot::RwLock;

pub fn channel<T>(size: usize) -> (Sender<T>, Receiver<T>)
where
    T: Clone,
{
    let inner = Arc::new(Inner::new(size));

    (
        Sender {
            inner: inner.clone(),
        },
        Receiver { inner },
    )
}

pub struct Sender<T>
where
    T: Clone,
{
    inner: Arc<Inner<T>>,
}

impl<T> Sender<T> where T: Clone {}

pub struct Receiver<T>
where
    T: Clone,
{
    inner: Arc<Inner<T>>,
}

struct Inner<T>
where
    T: Clone,
{
    /// Item buffer of the channel. This is always guaranteed to be same size as `size`.
    buffer: Box<[Slot<T>]>,

    /// Number of active Senders.
    senders: AtomicUsize,
}

impl<T> Inner<T>
where
    T: Clone,
{
    fn new(size: usize) -> Self {
        let mut buffer = Vec::with_capacity(size);
        for _ in 0..size {
            buffer.push(Slot::new());
        }

        Self {
            buffer: buffer.into_boxed_slice(),
            senders: AtomicUsize::new(0),
        }
    }
}

struct Slot<T>
where
    T: Clone,
{
    value: UnsafeCell<Option<T>>,
}

impl<T> Slot<T>
where
    T: Clone,
{
    fn new() -> Self {
        Self {
            value: UnsafeCell::new(None),
        }
    }
}
