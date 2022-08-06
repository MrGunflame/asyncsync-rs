use core::fmt::{self, Display, Formatter};

#[cfg(feature = "std")]
use std::error::Error;

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

#[cfg(feature = "std")]
impl Error for RecvError {}

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
impl Error for TryRecvError {}
