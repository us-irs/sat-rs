use std::collections::HashMap;
use std::sync::mpsc;

use derive_new::new;
use log::warn;
use satrs::hk::HkRequest;
use satrs::mode::ModeRequest;
use satrs::pus::action::ActionRequestWithId;
use satrs::pus::verification::{
    FailParams, TcStateAccepted, VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{GenericRoutingError, PusRequestRouter};
use satrs::queue::GenericSendError;
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusPacket;
use satrs::TargetId;
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
    pub(crate) target_id: TargetId,
    pub(crate) request: Request,
}

#[derive(Clone, Debug)]
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

impl GenericRequestRouter {
    fn handle_error_generic(
        &self,
        target_id: satrs::TargetId,
        token: satrs::pus::verification::VerificationToken<
            satrs::pus::verification::TcStateAccepted,
        >,
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
                    .start_failure(
                        token,
                        FailParams::new(time_stamp, &tmtc_err::UNKNOWN_TARGET_ID, &fail_data),
                    )
                    .expect("Sending start failure failed");
            }
            GenericRoutingError::SendError(_) => {
                let mut fail_data: [u8; 8] = [0; 8];
                fail_data.copy_from_slice(&target_id.to_be_bytes());
                verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(time_stamp, &tmtc_err::ROUTING_ERROR, &fail_data),
                    )
                    .expect("Sending start failure failed");
            }
            GenericRoutingError::NotEnoughAppData { expected, found } => {
                let mut context_info = (found as u32).to_be_bytes().to_vec();
                context_info.extend_from_slice(&(expected as u32).to_be_bytes());
                verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(time_stamp, &tmtc_err::NOT_ENOUGH_APP_DATA, &context_info),
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
    fn handle_error(
        &self,
        target_id: satrs::TargetId,
        token: satrs::pus::verification::VerificationToken<
            satrs::pus::verification::TcStateAccepted,
        >,
        tc: &PusTcReader,
        error: GenericRoutingError,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) {
        self.handle_error_generic(target_id, token, tc, error, time_stamp, verif_reporter)
    }
}

impl PusRequestRouter<ActionRequestWithId> for GenericRequestRouter {
    type Error = GenericRoutingError;

    fn route(
        &self,
        target_id: TargetId,
        action_request: ActionRequestWithId,
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
    fn handle_error(
        &self,
        target_id: satrs::TargetId,
        token: satrs::pus::verification::VerificationToken<
            satrs::pus::verification::TcStateAccepted,
        >,
        tc: &PusTcReader,
        error: GenericRoutingError,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) {
        self.handle_error_generic(target_id, token, tc, error, time_stamp, verif_reporter)
    }
}
