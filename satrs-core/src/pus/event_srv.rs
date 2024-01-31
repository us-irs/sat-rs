use crate::events::EventU32;
use crate::pool::SharedPool;
use crate::pus::event_man::{EventRequest, EventRequestWithToken};
use crate::pus::verification::{StdVerifReporterWithSender, TcStateToken};
use crate::pus::{
    EcssTcReceiver, EcssTmSender, PartialPusHandlingError, PusPacketHandlerResult,
    PusPacketHandlingError, PusServiceBaseWithStore,
};
use alloc::boxed::Box;
use spacepackets::ecss::event::Subservice;
use spacepackets::ecss::tc::PusTcReader;
use spacepackets::ecss::PusPacket;
use std::sync::mpsc::Sender;

pub struct PusService5EventHandler {
    psb: PusServiceBaseWithStore,
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
            psb: PusServiceBaseWithStore::new(
                tc_receiver,
                shared_tc_store,
                tm_sender,
                tm_apid,
                verification_handler,
            ),
            event_request_tx,
        }
    }

    pub fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        let possible_packet = self.psb.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(PusPacketHandlerResult::Empty);
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        self.psb
            .convert_possible_packet_to_tc_buf(&ecss_tc_and_token)?;
        let (tc, _) = PusTcReader::new(&self.psb.pus_buf)?;
        let subservice = tc.subservice();
        let srv = Subservice::try_from(subservice);
        if srv.is_err() {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                ecss_tc_and_token.token,
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
                .start_success(ecss_tc_and_token.token, Some(&stamp))
                .map_err(|_| PartialPusHandlingError::Verification);
            let partial_error = start_token.clone().err();
            let mut token: TcStateToken = ecss_tc_and_token.token.into();
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
        let time_stamp = self.psb.get_current_timestamp(&mut partial_error);
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
                    subservice,
                    ecss_tc_and_token.token,
                ));
            }
        }

        Ok(PusPacketHandlerResult::RequestHandled)
    }
}
