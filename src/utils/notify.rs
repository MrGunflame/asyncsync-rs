use core::marker::PhantomPinned;
use core::ptr::{addr_of_mut, NonNull};
use core::task::Waker;

use crate::linked_list::{Link, Pointers};

#[derive(Debug)]
pub struct Waiter {
    pub waker: Option<Waker>,
    pub notified: bool,

    pointers: Pointers<Self>,

    _pin: PhantomPinned,
}

impl Waiter {
    pub fn new() -> Self {
        Self {
            waker: None,
            notified: false,
            pointers: Pointers::new(),
            _pin: PhantomPinned,
        }
    }
}

unsafe impl Link for Waiter {
    unsafe fn pointers(ptr: NonNull<Self>) -> NonNull<Pointers<Self>> {
        let ptr = ptr.as_ptr();

        let pointers = addr_of_mut!((*ptr).pointers);
        NonNull::new_unchecked(pointers)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum State {
    Init,
    Pending,
    Done,
}
