#[cfg(feature = "std")]
use std::sync::mpsc;

use crate::ComponentId;

/// Generic channel ID type.
pub type ChannelId = u32;

/// Generic error type for sending something via a message queue.
#[derive(Debug, Copy, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenericSendError {
    #[error("rx side has disconnected")]
    RxDisconnected,
    #[error("queue with max capacity of {0:?} is full")]
    QueueFull(Option<u32>),
    #[error("target queue with ID {0} does not exist")]
    TargetDoesNotExist(ComponentId),
}

/// Generic error type for sending something via a message queue.
#[derive(Debug, Copy, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenericReceiveError {
    #[error("nothing to receive")]
    Empty,
    #[error("tx side with id {0:?} has disconnected")]
    TxDisconnected(Option<ComponentId>),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum GenericTargetedMessagingError {
    #[error("generic targeted messaging send error: {0}")]
    Send(#[from] GenericSendError),
    #[error("generic targeted messaging receive error: {0}")]
    Receive(#[from] GenericReceiveError),
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
