use core::fmt::{Display, Formatter};
#[cfg(feature = "std")]
use std::error::Error;
#[cfg(feature = "std")]
use std::sync::mpsc;

/// Generic error type for sending something via a message queue.
#[derive(Debug, Copy, Clone)]
pub enum GenericSendError {
    RxDisconnected,
    QueueFull(Option<u32>),
}

impl Display for GenericSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            GenericSendError::RxDisconnected => {
                write!(f, "rx side has disconnected")
            }
            GenericSendError::QueueFull(max_cap) => {
                write!(f, "queue with max capacity of {max_cap:?} is full")
            }
        }
    }
}

#[cfg(feature = "std")]
impl Error for GenericSendError {}

/// Generic error type for sending something via a message queue.
#[derive(Debug, Copy, Clone)]
pub enum GenericRecvError {
    Empty,
    TxDisconnected,
}

impl Display for GenericRecvError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TxDisconnected => {
                write!(f, "tx side has disconnected")
            }
            Self::Empty => {
                write!(f, "nothing to receive")
            }
        }
    }
}

#[cfg(feature = "std")]
impl Error for GenericRecvError {}

#[cfg(feature = "std")]
impl<T> From<mpsc::SendError<T>> for GenericSendError {
    fn from(_: mpsc::SendError<T>) -> Self {
        GenericSendError::RxDisconnected
    }
}

#[cfg(feature = "std")]
impl<T> From<mpsc::TrySendError<T>> for GenericSendError {
    fn from(err: mpsc::TrySendError<T>) -> Self {
        match err {
            mpsc::TrySendError::Full(_) => GenericSendError::QueueFull(None),
            mpsc::TrySendError::Disconnected(_) => GenericSendError::RxDisconnected,
        }
    }
}

#[cfg(feature = "crossbeam")]
impl<T> From<crossbeam_channel::SendError<T>> for GenericSendError {
    fn from(_: crossbeam_channel::SendError<T>) -> Self {
        GenericSendError::RxDisconnected
    }
}

#[cfg(feature = "crossbeam")]
impl<T> From<crossbeam_channel::TrySendError<T>> for GenericSendError {
    fn from(err: crossbeam_channel::TrySendError<T>) -> Self {
        match err {
            crossbeam_channel::TrySendError::Full(_) => GenericSendError::QueueFull(None),
            crossbeam_channel::TrySendError::Disconnected(_) => GenericSendError::RxDisconnected,
        }
    }
}
