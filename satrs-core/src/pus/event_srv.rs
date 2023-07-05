use crate::events::EventU32;
use crate::pool::{SharedPool, StoreAddr};
use crate::pus::event_man::{EventRequest, EventRequestWithToken};
use crate::pus::verification::{
    StdVerifReporterWithSender, TcStateAccepted, TcStateToken, VerificationToken,
};
use crate::pus::{
    AcceptedTc, PartialPusHandlingError, PusPacketHandlerResult, PusPacketHandlingError,
    PusServiceBase, PusServiceHandler,
};
use crate::tmtc::tm_helper::SharedTmStore;
use spacepackets::ecss::event::Subservice;
use spacepackets::ecss::PusPacket;
use spacepackets::tc::PusTc;
use std::format;
use std::sync::mpsc::{Receiver, Sender};

pub struct PusService5EventHandler {
    psb: PusServiceBase,
    event_request_tx: Sender<EventRequestWithToken>,
}

impl PusService5EventHandler {
    pub fn new(
        receiver: Receiver<AcceptedTc>,
        tc_pool: SharedPool,
        tm_tx: Sender<StoreAddr>,
        tm_store: SharedTmStore,
        tm_apid: u16,
        verification_handler: StdVerifReporterWithSender,
        event_request_tx: Sender<EventRequestWithToken>,
    ) -> Self {
        Self {
            psb: PusServiceBase::new(
                receiver,
                tc_pool,
                tm_tx,
                tm_store,
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
        {
            // Keep locked section as short as possible.
            let mut tc_pool = self
                .psb
                .tc_store
                .write()
                .map_err(|e| PusPacketHandlingError::RwGuardError(format!("{e}")))?;
            let tc_guard = tc_pool.read_with_guard(addr);
            let tc_raw = tc_guard.read().unwrap();
            self.psb.pus_buf[0..tc_raw.len()].copy_from_slice(tc_raw);
        }
        let (tc, _) = PusTc::from_bytes(&self.psb.pus_buf).unwrap();
        let srv = Subservice::try_from(tc.subservice());
        if srv.is_err() {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                token,
            ));
        }
        let mut handle_enable_disable_request = |enable: bool| {
            if tc.user_data().is_none() || tc.user_data().unwrap().len() < 4 {
                return Err(PusPacketHandlingError::NotEnoughAppData(
                    "At least 4 bytes event ID expected".into(),
                ));
            }
            let user_data = tc.user_data().unwrap();
            let event_u32 = EventU32::from(u32::from_be_bytes(user_data[0..4].try_into().unwrap()));

            let start_token = self
                .psb
                .verification_handler
                .start_success(token, Some(&self.psb.stamp_buf))
                .map_err(|_| PartialPusHandlingError::VerificationError);
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
                    PusPacketHandlingError::SendError("Forwarding event request failed".into())
                })?;
            if let Some(partial_error) = partial_error {
                return Ok(PusPacketHandlerResult::RequestHandledPartialSuccess(
                    partial_error,
                ));
            }
            Ok(PusPacketHandlerResult::RequestHandled)
        };
        match srv.unwrap() {
            Subservice::TmInfoReport
            | Subservice::TmLowSeverityReport
            | Subservice::TmMediumSeverityReport
            | Subservice::TmHighSeverityReport => {
                return Err(PusPacketHandlingError::InvalidSubservice(tc.subservice()))
            }
            Subservice::TcEnableEventGeneration => {
                handle_enable_disable_request(true)?;
            }
            Subservice::TcDisableEventGeneration => {
                handle_enable_disable_request(false)?;
            }
            Subservice::TcReportDisabledList | Subservice::TmDisabledEventsReport => {
                return Ok(PusPacketHandlerResult::SubserviceNotImplemented(
                    tc.subservice(),
                    token,
                ));
            }
        }

        Ok(PusPacketHandlerResult::RequestHandled)
    }
}
