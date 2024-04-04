use core::fmt::{Display, Formatter};
#[cfg(feature = "std")]
use std::error::Error;
#[cfg(feature = "std")]
use std::sync::mpsc;

use crate::ComponentId;

/// Generic channel ID type.
pub type ChannelId = u32;

/// Generic error type for sending something via a message queue.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GenericSendError {
    RxDisconnected,
    QueueFull(Option<u32>),
    TargetDoesNotExist(ComponentId),
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
            GenericSendError::TargetDoesNotExist(target) => {
                write!(f, "target queue with ID {target} does not exist")
            }
        }
    }
}

#[cfg(feature = "std")]
impl Error for GenericSendError {}

/// Generic error type for sending something via a message queue.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GenericReceiveError {
    Empty,
    TxDisconnected(Option<ComponentId>),
}

impl Display for GenericReceiveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TxDisconnected(channel_id) => {
                write!(f, "tx side with id {channel_id:?} has disconnected")
            }
            Self::Empty => {
                write!(f, "nothing to receive")
            }
        }
    }
}

#[cfg(feature = "std")]
impl Error for GenericReceiveError {}

#[derive(Debug, Clone)]
pub enum GenericTargetedMessagingError {
    Send(GenericSendError),
    Receive(GenericReceiveError),
}
impl From<GenericSendError> for GenericTargetedMessagingError {
    fn from(value: GenericSendError) -> Self {
        Self::Send(value)
    }
}

impl From<GenericReceiveError> for GenericTargetedMessagingError {
    fn from(value: GenericReceiveError) -> Self {
        Self::Receive(value)
    }
}

impl Display for GenericTargetedMessagingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Send(err) => write!(f, "generic targeted messaging error: {}", err),
            Self::Receive(err) => write!(f, "generic targeted messaging error: {}", err),
        }
    }
}

#[cfg(feature = "std")]
impl Error for GenericTargetedMessagingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            GenericTargetedMessagingError::Send(send) => Some(send),
            GenericTargetedMessagingError::Receive(receive) => Some(receive),
        }
    }
}

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
