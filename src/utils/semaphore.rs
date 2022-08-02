use core::cell::UnsafeCell;
use core::marker::PhantomPinned;
use core::ptr::NonNull;
use core::task::Waker;

use crate::linked_list::Link;

#[derive(Debug)]
pub struct Waiter(UnsafeCell<WaiterInner>);

impl Waiter {
    pub fn new() -> Self {
        Self(UnsafeCell::new(WaiterInner {
            waker: None,

            _pin: PhantomPinned,
            next: None,
            prev: None,
        }))
    }

    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get(&self) -> &mut WaiterInner {
        &mut *self.0.get()
    }
}

#[derive(Debug)]
pub struct WaiterInner {
    pub waker: Option<Waker>,

    _pin: PhantomPinned,

    next: Option<NonNull<Waiter>>,
    prev: Option<NonNull<Waiter>>,
}

/// # Safety
///
/// Waiter is pinned.
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// New state requesting n permits.
    Init(usize),
    /// Waiting for n permits.
    Waiting(usize),
    Done,
}
