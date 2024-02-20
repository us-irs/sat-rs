use std::collections::HashMap;
use std::sync::mpsc;

use derive_new::new;
use satrs::action::ActionRequest;
use satrs::hk::HkRequest;
use satrs::mode::ModeRequest;
use satrs::pus::action::PusActionRequestRouter;
use satrs::pus::hk::PusHkRequestRouter;
use satrs::pus::verification::{TcStateAccepted, VerificationToken};
use satrs::pus::GenericRoutingError;
use satrs::queue::GenericSendError;
use satrs::TargetId;

#[allow(dead_code)]
#[derive(Clone, Eq, PartialEq, Debug)]
#[non_exhaustive]
pub enum Request {
    Hk(HkRequest),
    Mode(ModeRequest),
    Action(ActionRequest),
}

#[derive(Clone, Eq, PartialEq, Debug, new)]
pub struct TargetedRequest {
    pub(crate) target_id: TargetId,
    pub(crate) request: Request,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct RequestWithToken {
    pub(crate) targeted_request: TargetedRequest,
    pub(crate) token: VerificationToken<TcStateAccepted>,
}

impl RequestWithToken {
    pub fn new(
        target_id: TargetId,
        request: Request,
        token: VerificationToken<TcStateAccepted>,
    ) -> Self {
        Self {
            targeted_request: TargetedRequest::new(target_id, request),
            token,
        }
    }
}

#[derive(Default, Clone)]
pub struct GenericRequestRouter(pub HashMap<TargetId, mpsc::Sender<RequestWithToken>>);

impl PusHkRequestRouter for GenericRequestRouter {
    type Error = GenericRoutingError;

    fn route(
        &self,
        target_id: TargetId,
        hk_request: HkRequest,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<(), Self::Error> {
        if let Some(sender) = self.0.get(&target_id) {
            sender
                .send(RequestWithToken::new(
                    target_id,
                    Request::Hk(hk_request),
                    token,
                ))
                .map_err(|_| GenericRoutingError::SendError(GenericSendError::RxDisconnected))?;
        }
        Ok(())
    }
}

impl PusActionRequestRouter for GenericRequestRouter {
    type Error = GenericRoutingError;

    fn route(
        &self,
        target_id: TargetId,
        action_request: ActionRequest,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<(), Self::Error> {
        if let Some(sender) = self.0.get(&target_id) {
            sender
                .send(RequestWithToken::new(
                    target_id,
                    Request::Action(action_request),
                    token,
                ))
                .map_err(|_| GenericRoutingError::SendError(GenericSendError::RxDisconnected))?;
        }
        Ok(())
    }
}
