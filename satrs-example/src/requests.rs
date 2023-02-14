use satrs_core::hk::HkRequest;
use satrs_core::pus::verification::{TcStateAccepted, VerificationToken};

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Request {
    HkRequest(HkRequest),
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct RequestWithToken(pub Request, pub VerificationToken<TcStateAccepted>);
