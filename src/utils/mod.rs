pub mod notify;
pub mod semaphore;

#[inline]
pub(crate) fn is_unpin<T: Unpin>() {}
