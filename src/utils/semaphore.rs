use core::marker::PhantomPinned;
use core::ptr::addr_of_mut;
use core::ptr::NonNull;
use core::task::Waker;

use crate::linked_list::{Link, Pointers};

#[derive(Debug)]
pub struct Waiter {
    pub waker: Option<Waker>,
    pub permits: usize,

    pointers: Pointers<Self>,

    _pin: PhantomPinned,
}

impl Waiter {
    pub fn new(permits: usize) -> Self {
        Self {
            waker: None,
            permits,
            pointers: Pointers::new(),
            _pin: PhantomPinned,
        }
    }
}

/// # Safety
///
/// Waiter is pinned.
unsafe impl Link for Waiter {
    unsafe fn pointers(ptr: NonNull<Self>) -> NonNull<Pointers<Self>> {
        let ptr = ptr.as_ptr();

        let pointers = addr_of_mut!((*ptr).pointers);
        NonNull::new_unchecked(pointers)
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
