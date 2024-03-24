use std::collections::HashMap;
use std::sync::mpsc;

use log::warn;
use satrs::action::ActionRequest;
use satrs::hk::HkRequest;
use satrs::pus::verification::{
    FailParams, TcStateAccepted, VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{ActiveRequestProvider, GenericRoutingError, PusRequestRouter};
use satrs::queue::GenericSendError;
use satrs::request::{GenericMessage, RequestId};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusPacket;
use satrs::ComponentId;
use satrs_example::config::tmtc_err;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum CompositeRequest {
    Hk(HkRequest),
    Action(ActionRequest),
}

//#[derive(Clone, Debug)]
// pub struct CompositeRequestWithToken {
//pub(crate) targeted_request: GenericMessage<CompositeRequest>,
//}

/*
impl CompositeRequestWithToken {
    pub fn new(
        target_id: ComponentId,
        request_id: RequestId,
        request: CompositeRequest,
        token: VerificationToken<TcStateStarted>,
    ) -> Self {
        Self {
            targeted_request: GenericMessage::new(request_id, target_id, request),
            token,
        }
    }
}
*/

#[derive(Default, Clone)]
pub struct GenericRequestRouter(
    pub HashMap<ComponentId, mpsc::Sender<GenericMessage<CompositeRequest>>>,
);

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
        let accepted_token: VerificationToken<TcStateAccepted> = active_request
            .token()
            .try_into()
            .expect("token is not in accepted state");
        match error {
            GenericRoutingError::UnknownTargetId(id) => {
                let mut fail_data: [u8; 8] = [0; 8];
                fail_data.copy_from_slice(&id.to_be_bytes());
                verif_reporter
                    .completion_failure(
                        accepted_token,
                        FailParams::new(time_stamp, &tmtc_err::UNKNOWN_TARGET_ID, &fail_data),
                    )
                    .expect("Sending start failure failed");
            }
            GenericRoutingError::Send(_) => {
                let mut fail_data: [u8; 8] = [0; 8];
                fail_data.copy_from_slice(&active_request.target_id().to_be_bytes());
                verif_reporter
                    .completion_failure(
                        accepted_token,
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
        request_id: RequestId,
        source_id: ComponentId,
        target_id: ComponentId,
        hk_request: HkRequest,
    ) -> Result<(), Self::Error> {
        if let Some(sender) = self.0.get(&target_id) {
            sender
                .send(GenericMessage::new(
                    request_id,
                    source_id,
                    CompositeRequest::Hk(hk_request),
                ))
                .map_err(|_| GenericRoutingError::Send(GenericSendError::RxDisconnected))?;
            return Ok(());
        }
        Err(GenericRoutingError::UnknownTargetId(target_id))
    }
}

impl PusRequestRouter<ActionRequest> for GenericRequestRouter {
    type Error = GenericRoutingError;

    fn route(
        &self,
        request_id: RequestId,
        source_id: ComponentId,
        target_id: ComponentId,
        action_request: ActionRequest,
    ) -> Result<(), Self::Error> {
        if let Some(sender) = self.0.get(&target_id) {
            sender
                .send(GenericMessage::new(
                    request_id,
                    source_id,
                    CompositeRequest::Action(action_request),
                ))
                .map_err(|_| GenericRoutingError::Send(GenericSendError::RxDisconnected))?;
            return Ok(());
        }
        Err(GenericRoutingError::UnknownTargetId(target_id))
    }
}
