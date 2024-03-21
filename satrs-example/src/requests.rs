use std::collections::HashMap;
use std::sync::mpsc;

use derive_new::new;
use log::warn;
use satrs::hk::HkRequest;
use satrs::mode::ModeRequest;
use satrs::pus::action::ActionRequestWithId;
use satrs::pus::verification::{
    FailParams, TcStateStarted, VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{ActiveRequestProvider, GenericRoutingError, PusRequestRouter};
use satrs::queue::GenericSendError;
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusPacket;
use satrs::ComponentId;
use satrs_example::config::tmtc_err;

#[allow(dead_code)]
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Request {
    Hk(HkRequest),
    Mode(ModeRequest),
    Action(ActionRequestWithId),
}

#[derive(Clone, Debug, new)]
pub struct TargetedRequest {
    pub(crate) target_id: ComponentId,
    pub(crate) request: Request,
}

#[derive(Clone, Debug)]
pub struct RequestWithToken {
    pub(crate) targeted_request: TargetedRequest,
    pub(crate) token: VerificationToken<TcStateStarted>,
}

impl RequestWithToken {
    pub fn new(
        target_id: ComponentId,
        request: Request,
        token: VerificationToken<TcStateStarted>,
    ) -> Self {
        Self {
            targeted_request: TargetedRequest::new(target_id, request),
            token,
        }
    }
}

#[derive(Default, Clone)]
pub struct GenericRequestRouter(pub HashMap<ComponentId, mpsc::Sender<RequestWithToken>>);

impl GenericRequestRouter {
    pub(crate) fn handle_error_generic(
        &self,
        active_request: &impl ActiveRequestProvider,
        tc: &PusTcReader,
        error: GenericRoutingError,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) {
        warn!(
            "Routing request for service {} failed: {error:?}",
            tc.service()
        );
        match error {
            GenericRoutingError::UnknownTargetId(id) => {
                let mut fail_data: [u8; 8] = [0; 8];
                fail_data.copy_from_slice(&id.to_be_bytes());
                verif_reporter
                    .completion_failure(
                        active_request.token(),
                        FailParams::new(time_stamp, &tmtc_err::UNKNOWN_TARGET_ID, &fail_data),
                    )
                    .expect("Sending start failure failed");
            }
            GenericRoutingError::Send(_) => {
                let mut fail_data: [u8; 8] = [0; 8];
                fail_data.copy_from_slice(&active_request.target_id().to_be_bytes());
                verif_reporter
                    .completion_failure(
                        active_request.token(),
                        FailParams::new(time_stamp, &tmtc_err::ROUTING_ERROR, &fail_data),
                    )
                    .expect("Sending start failure failed");
            }
        }
    }
}
impl PusRequestRouter<HkRequest> for GenericRequestRouter {
    type Error = GenericRoutingError;

    fn route(
        &self,
        target_id: ComponentId,
        hk_request: HkRequest,
        token: VerificationToken<TcStateStarted>,
    ) -> Result<(), Self::Error> {
        if let Some(sender) = self.0.get(&target_id) {
            sender
                .send(RequestWithToken::new(
                    target_id,
                    Request::Hk(hk_request),
                    token,
                ))
                .map_err(|_| GenericRoutingError::Send(GenericSendError::RxDisconnected))?;
        }
        Ok(())
    }
}

impl PusRequestRouter<ActionRequestWithId> for GenericRequestRouter {
    type Error = GenericRoutingError;

    fn route(
        &self,
        target_id: ComponentId,
        action_request: ActionRequestWithId,
        token: VerificationToken<TcStateStarted>,
    ) -> Result<(), Self::Error> {
        if let Some(sender) = self.0.get(&target_id) {
            sender
                .send(RequestWithToken::new(
                    target_id,
                    Request::Action(action_request),
                    token,
                ))
                .map_err(|_| GenericRoutingError::Send(GenericSendError::RxDisconnected))?;
        }
        Ok(())
    }
}
