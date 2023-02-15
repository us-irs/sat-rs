use satrs_core::hk::HkRequest;
use satrs_core::mode::ModeRequest;
use satrs_core::pus::verification::{TcStateAccepted, VerificationToken};

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
#[non_exhaustive]
pub enum Request {
    HkRequest(HkRequest),
    ModeRequest(ModeRequest),
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct RequestWithToken(pub Request, pub VerificationToken<TcStateAccepted>);
