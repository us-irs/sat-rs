use crate::tmtc::AddressableId;

pub type CollectionIntervalFactor = u32;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HkRequest {
    OneShot(AddressableId),
    Enable(AddressableId),
    Disable(AddressableId),
    ModifyCollectionInterval(AddressableId, CollectionIntervalFactor),
}
