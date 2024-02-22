use core::fmt::{Display, Formatter};
#[cfg(feature = "std")]
use std::error::Error;

/// Generic channel ID type.
pub type ChannelId = u32;

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
pub enum GenericReceiveError {
    Empty,
    TxDisconnected(Option<ChannelId>),
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
    TargetDoesNotExist(ChannelId),
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
