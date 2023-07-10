use crate::events::EventU32;
use crate::pool::{SharedPool, StoreAddr};
use crate::pus::event_man::{EventRequest, EventRequestWithToken};
use crate::pus::verification::{
    StdVerifReporterWithSender, TcStateAccepted, TcStateToken, VerificationToken,
};
use crate::pus::{
    EcssTcReceiver, EcssTmSender, PartialPusHandlingError, PusPacketHandlerResult,
    PusPacketHandlingError, PusServiceBase, PusServiceHandler,
};
use spacepackets::ecss::event::Subservice;
use spacepackets::ecss::tc::PusTc;
use spacepackets::ecss::PusPacket;
use std::boxed::Box;
use std::sync::mpsc::Sender;

pub struct PusService5EventHandler {
    psb: PusServiceBase,
    event_request_tx: Sender<EventRequestWithToken>,
}

impl PusService5EventHandler {
    pub fn new(
        tc_receiver: Box<dyn EcssTcReceiver>,
        shared_tc_store: SharedPool,
        tm_sender: Box<dyn EcssTmSender>,
        tm_apid: u16,
        verification_handler: StdVerifReporterWithSender,
        event_request_tx: Sender<EventRequestWithToken>,
    ) -> Self {
        Self {
            psb: PusServiceBase::new(
                tc_receiver,
                shared_tc_store,
                tm_sender,
                tm_apid,
                verification_handler,
            ),
            event_request_tx,
        }
    }
}

impl PusServiceHandler for PusService5EventHandler {
    fn psb_mut(&mut self) -> &mut PusServiceBase {
        &mut self.psb
    }
    fn psb(&self) -> &PusServiceBase {
        &self.psb
    }

    fn handle_one_tc(
        &mut self,
        addr: StoreAddr,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        self.copy_tc_to_buf(addr)?;
        let (tc, _) = PusTc::from_bytes(&self.psb.pus_buf)?;
        let subservice = tc.subservice();
        let srv = Subservice::try_from(subservice);
        if srv.is_err() {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                token,
            ));
        }
        let handle_enable_disable_request = |enable: bool, stamp: [u8; 7]| {
            if tc.user_data().len() < 4 {
                return Err(PusPacketHandlingError::NotEnoughAppData(
                    "At least 4 bytes event ID expected".into(),
                ));
            }
            let user_data = tc.user_data();
            let event_u32 = EventU32::from(u32::from_be_bytes(user_data[0..4].try_into().unwrap()));
            let start_token = self
                .psb
                .verification_handler
                .borrow_mut()
                .start_success(token, Some(&stamp))
                .map_err(|_| PartialPusHandlingError::Verification);
            let partial_error = start_token.clone().err();
            let mut token: TcStateToken = token.into();
            if let Ok(start_token) = start_token {
                token = start_token.into();
            }
            let event_req_with_token = if enable {
                EventRequestWithToken {
                    request: EventRequest::Enable(event_u32),
                    token,
                }
            } else {
                EventRequestWithToken {
                    request: EventRequest::Disable(event_u32),
                    token,
                }
            };
            self.event_request_tx
                .send(event_req_with_token)
                .map_err(|_| {
                    PusPacketHandlingError::Other("Forwarding event request failed".into())
                })?;
            if let Some(partial_error) = partial_error {
                return Ok(PusPacketHandlerResult::RequestHandledPartialSuccess(
                    partial_error,
                ));
            }
            Ok(PusPacketHandlerResult::RequestHandled)
        };
        let mut partial_error = None;
        let time_stamp = self.psb().get_current_timestamp(&mut partial_error);
        match srv.unwrap() {
            Subservice::TmInfoReport
            | Subservice::TmLowSeverityReport
            | Subservice::TmMediumSeverityReport
            | Subservice::TmHighSeverityReport => {
                return Err(PusPacketHandlingError::InvalidSubservice(tc.subservice()))
            }
            Subservice::TcEnableEventGeneration => {
                handle_enable_disable_request(true, time_stamp)?;
            }
            Subservice::TcDisableEventGeneration => {
                handle_enable_disable_request(false, time_stamp)?;
            }
            Subservice::TcReportDisabledList | Subservice::TmDisabledEventsReport => {
                return Ok(PusPacketHandlerResult::SubserviceNotImplemented(
                    subservice, token,
                ));
            }
        }

        Ok(PusPacketHandlerResult::RequestHandled)
    }
}
