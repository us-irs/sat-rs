use satrs_core::hk::HkRequest;
use satrs_core::mode::ModeRequest;
use satrs_core::pus::verification::{TcStateAccepted, VerificationToken};
use satrs_core::tmtc::TargetId;

#[allow(dead_code)]
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum ActionRequest {
    CmdWithU32Id((u32, Vec<u8>)),
    CmdWithStringId((String, Vec<u8>)),
}

#[allow(dead_code)]
#[derive(Clone, Eq, PartialEq, Debug)]
#[non_exhaustive]
pub enum Request {
    Hk(HkRequest),
    Mode(ModeRequest),
    Action(ActionRequest),
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct TargetedRequest {
    pub(crate) target_id: TargetId,
    pub(crate) request: Request,
}

impl TargetedRequest {
    pub fn new(target_id: TargetId, request: Request) -> Self {
        Self { target_id, request }
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct RequestWithToken {
    pub(crate) targeted_request: TargetedRequest,
    pub(crate) token: VerificationToken<TcStateAccepted>,
}

impl RequestWithToken {
    pub fn new(
        target_id: u32,
        request: Request,
        token: VerificationToken<TcStateAccepted>,
    ) -> Self {
        Self {
            targeted_request: TargetedRequest::new(target_id, request),
            token,
        }
    }
}
