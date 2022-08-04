//! Synchronization primitives single-threaded contexts.
//!
//! This module provides provides the same synchronization primitives, but for a single-threaded
//! context.
mod notify;

pub use notify::{Notified, Notify};
