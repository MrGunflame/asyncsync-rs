pub mod notify;

#[inline]
pub(crate) fn is_unpin<T: Unpin>() {}
