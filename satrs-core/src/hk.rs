use crate::tmtc::AddressableId;

pub type CollectionIntervalFactor = u32;
pub type UniqueId = u32;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HkRequest {
    OneShot(UniqueId),
    Enable(UniqueId),
    Disable(UniqueId),
    ModifyCollectionInterval(UniqueId, CollectionIntervalFactor),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TargetedHkRequest {
    target: u32,
    hk_request: HkRequest
}
