use crate::hk::HkRequest;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Request {
    HkRequest(HkRequest),
}
