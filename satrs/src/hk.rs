use crate::ComponentId;

pub type CollectionIntervalFactor = u32;
/// Unique Identifier for a certain housekeeping dataset.
pub type UniqueId = u32;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct HkRequest {
    pub unique_id: UniqueId,
    pub variant: HkRequestVariant,
}

impl HkRequest {
    pub fn new(unique_id: UniqueId, variant: HkRequestVariant) -> Self {
        Self { unique_id, variant }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HkRequestVariant {
    OneShot,
    EnablePeriodic,
    DisablePeriodic,
    ModifyCollectionInterval(CollectionIntervalFactor),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TargetedHkRequest {
    pub target_id: ComponentId,
    pub hk_request: HkRequestVariant,
}

impl TargetedHkRequest {
    pub fn new(target_id: ComponentId, hk_request: HkRequestVariant) -> Self {
        Self {
            target_id,
            hk_request,
        }
    }
}
