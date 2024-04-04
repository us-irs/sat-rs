use crate::{params::Params, pool::StoreAddr};

#[cfg(feature = "alloc")]
pub use alloc_mod::*;

pub type ActionId = u32;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct ActionRequest {
    pub action_id: ActionId,
    pub variant: ActionRequestVariant,
}

impl ActionRequest {
    pub fn new(action_id: ActionId, variant: ActionRequestVariant) -> Self {
        Self { action_id, variant }
    }
}

#[non_exhaustive]
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum ActionRequestVariant {
    NoData,
    StoreData(StoreAddr),
    #[cfg(feature = "alloc")]
    VecData(alloc::vec::Vec<u8>),
}

#[derive(Debug, PartialEq, Clone)]
pub struct ActionReply {
    pub action_id: ActionId,
    pub variant: ActionReplyVariant,
}

/// A reply to an action request.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ActionReplyVariant {
    CompletionFailed(Params),
    StepFailed { step: u32, reason: Params },
    Completed,
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use super::*;

    #[derive(Debug, Eq, PartialEq, Clone)]
    pub struct ActionRequestStringId {
        pub action_id: alloc::string::String,
        pub variant: ActionRequestVariant,
    }

    impl ActionRequestStringId {
        pub fn new(action_id: alloc::string::String, variant: ActionRequestVariant) -> Self {
            Self { action_id, variant }
        }
    }

    #[derive(Debug, PartialEq, Clone)]
    pub struct ActionReplyStringId {
        pub action_id: alloc::string::String,
        pub variant: ActionReplyVariant,
    }
}

#[cfg(test)]
mod tests {}
