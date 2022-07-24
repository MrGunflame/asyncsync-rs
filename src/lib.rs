//! Runtime-independent synchronization primitives for asynchronous Rust.
mod linked_list;
mod notify;

#[cfg(feature = "local")]
pub mod local;

pub use notify::{Notified, Notify};

#[inline]
pub(crate) fn is_unpin<T: Unpin>() {}
