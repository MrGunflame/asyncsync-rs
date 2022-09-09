//! Runtime-independent synchronization primitives for asynchronous Rust.
#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(elided_lifetimes_in_paths)]
#![deny(missing_debug_implementations)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(any(feature = "std", test))]
extern crate std;

mod linked_list;
pub(crate) mod utils;

pub mod errors;

#[cfg(feature = "std")]
mod notify;

#[cfg(feature = "std")]
pub mod semaphore;

#[cfg(feature = "std")]
pub mod channel;

#[cfg(feature = "local")]
pub mod local;

#[cfg(feature = "std")]
pub use notify::{Notified, Notify};

#[cfg(feature = "std")]
pub use semaphore::{Acquire, Permit, Semaphore};
