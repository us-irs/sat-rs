use crate::{pool::StoreAddr, TargetId};

pub type ActionId = u32;

#[non_exhaustive]
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum ActionRequest {
    UnsignedIdAndStoreData {
        action_id: ActionId,
        data_addr: StoreAddr,
    },
    #[cfg(feature = "alloc")]
    UnsignedIdAndVecData {
        action_id: ActionId,
        data: alloc::vec::Vec<u8>,
    },
    #[cfg(feature = "alloc")]
    StringIdAndVecData {
        action_id: alloc::string::String,
        data: alloc::vec::Vec<u8>,
    },
    #[cfg(feature = "alloc")]
    StringIdAndStoreData {
        action_id: alloc::string::String,
        data: StoreAddr,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetedActionRequest {
    target: TargetId,
    action_request: ActionRequest,
}

impl TargetedActionRequest {
    pub fn new(target: TargetId, action_request: ActionRequest) -> Self {
        Self {
            target,
            action_request,
        }
    }
}

/// A reply to an action request.
#[non_exhaustive]
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum ActionReply {
    CompletionFailed(ActionId),
    StepFailed {
        id: ActionId,
        step: u32,
    },
    Completed(ActionId),
    #[cfg(feature = "alloc")]
    CompletedStringId(alloc::string::String),
    #[cfg(feature = "alloc")]
    CompletionFailedStringId(alloc::string::String),
    #[cfg(feature = "alloc")]
    StepFailedStringId {
        id: alloc::string::String,
        step: u32,
    },
}
