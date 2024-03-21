use crate::ComponentId;

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
    pub target_id: ComponentId,
    pub hk_request: HkRequest,
}

impl TargetedHkRequest {
    pub fn new(target_id: ComponentId, hk_request: HkRequest) -> Self {
        Self {
            target_id,
            hk_request,
        }
    }
}
