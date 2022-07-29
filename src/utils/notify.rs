use core::{cell::UnsafeCell, marker::PhantomPinned, ptr::NonNull, task::Waker};

use crate::linked_list::Link;

#[derive(Debug)]
pub struct Waiter(UnsafeCell<WaiterInner>);

impl Waiter {
    pub fn new() -> Self {
        Self(UnsafeCell::new(WaiterInner {
            waker: None,
            notified: false,
            _pin: PhantomPinned,
            next: None,
            prev: None,
        }))
    }

    /// Returns a mutable reference to the contained [`WaiterInner`].
    ///
    /// # Safety
    ///
    /// The [`Waiter`] must be accessed from multiple threads at the same time.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get(&self) -> &mut WaiterInner {
        &mut *self.0.get()
    }
}

unsafe impl Link for Waiter {
    fn next(&self) -> Option<NonNull<Self>> {
        unsafe { self.get().next }
    }

    fn prev(&self) -> Option<NonNull<Self>> {
        unsafe { self.get().prev }
    }

    fn next_mut(&mut self) -> &mut Option<NonNull<Self>> {
        unsafe { &mut self.get().next }
    }

    fn prev_mut(&mut self) -> &mut Option<NonNull<Self>> {
        unsafe { &mut self.get().prev }
    }
}

#[derive(Debug)]
pub struct WaiterInner {
    pub waker: Option<Waker>,
    pub notified: bool,

    _pin: PhantomPinned,

    next: Option<NonNull<Waiter>>,
    prev: Option<NonNull<Waiter>>,
}

#[derive(Debug, PartialEq)]
pub enum State {
    Init,
    Pending,
    Done,
}
