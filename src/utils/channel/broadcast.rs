use core::marker::PhantomPinned;
use core::ptr::{addr_of_mut, NonNull};
use core::task::Waker;

use crate::linked_list::{Link, Pointers};

#[derive(Debug)]
pub(crate) struct Waiter {
    pub waker: Option<Waker>,
    pointers: Pointers<Self>,
    _pin: PhantomPinned,
}

unsafe impl Link for Waiter {
    unsafe fn pointers(ptr: NonNull<Self>) -> NonNull<Pointers<Self>> {
        let ptr = ptr.as_ptr();

        let pointers = addr_of_mut!((*ptr).pointers);

        // SAFETY: The new pointer has a positive offset from ptr.
        unsafe { NonNull::new_unchecked(pointers) }
    }
}
