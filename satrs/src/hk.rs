use crate::{
    pus::verification::{TcStateAccepted, VerificationToken},
    TargetId,
};

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
    pub target_id: TargetId,
    pub hk_request: HkRequest,
}

impl TargetedHkRequest {
    pub fn new(target_id: TargetId, hk_request: HkRequest) -> Self {
        Self {
            target_id,
            hk_request,
        }
    }
}

pub trait PusHkRequestRouter {
    type Error;
    fn route(
        &self,
        target_id: TargetId,
        hk_request: HkRequest,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<(), Self::Error>;
}
