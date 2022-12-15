use core::fmt::{self, Display, Formatter};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TrySendError<T> {
    Full(T),
    Closed(T),
}

impl<T> TrySendError<T> {
    pub const fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }

    pub const fn is_closed(&self) -> bool {
        matches!(self, Self::Closed(_))
    }

    pub const fn into_inner(self) -> T {
        match self {
            Self::Full(val) => val,
            Self::Closed(val) => val,
        }
    }
}

impl<T> Display for TrySendError<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => f.write_str("channel full"),
            Self::Closed(_) => f.write_str("channel closed"),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecvError {
    _priv: (),
}

impl RecvError {
    pub(crate) fn new() -> Self {
        Self { _priv: () }
    }
}

impl Display for RecvError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("channel closed")
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TryRecvError {
    Empty,
    Closed,
}

impl Display for TryRecvError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("channel empty"),
            Self::Closed => f.write_str("channel closed"),
        }
    }
}

#[cfg(feature = "std")]
mod _std_error {
    use std::error::Error;
    use std::fmt::Debug;

    use super::{RecvError, TryRecvError, TrySendError};

    impl<T: Debug> Error for TrySendError<T> {}
    impl Error for TryRecvError {}
    impl Error for RecvError {}
}
