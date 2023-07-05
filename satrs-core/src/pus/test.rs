use crate::pool::{SharedPool, StoreAddr};
use crate::pus::verification::{StdVerifReporterWithSender, TcStateAccepted, VerificationToken};
use crate::pus::{
    AcceptedTc, PartialPusHandlingError, PusPacketHandlerResult, PusPacketHandlingError,
    PusServiceBase, PusServiceHandler,
};
use crate::tmtc::tm_helper::SharedTmStore;
use spacepackets::ecss::PusPacket;
use spacepackets::tc::PusTc;
use spacepackets::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::SpHeader;
use std::format;
use std::sync::mpsc::{Receiver, Sender};

/// This is a helper class for [std] environments to handle generic PUS 17 (test service) packets.
/// This handler only processes ping requests and generates a ping reply for them accordingly.
pub struct PusService17TestHandler {
    psb: PusServiceBase,
}

impl PusService17TestHandler {
    pub fn new(
        receiver: Receiver<AcceptedTc>,
        tc_pool: SharedPool,
        tm_tx: Sender<StoreAddr>,
        tm_store: SharedTmStore,
        tm_apid: u16,
        verification_handler: StdVerifReporterWithSender,
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
        }
    }
}

impl PusServiceHandler for PusService17TestHandler {
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
        if tc.service() != 17 {
            return Err(PusPacketHandlingError::WrongService(tc.service()));
        }
        if tc.subservice() == 1 {
            let mut partial_error = None;
            let time_stamp = self.psb().get_current_timestamp(&mut partial_error);
            let result = self
                .psb
                .verification_handler
                .get_mut()
                .start_success(token, Some(&time_stamp))
                .map_err(|_| PartialPusHandlingError::VerificationError);
            let start_token = if let Ok(result) = result {
                Some(result)
            } else {
                partial_error = Some(result.unwrap_err());
                None
            };
            // Sequence count will be handled centrally in TM funnel.
            let mut reply_header = SpHeader::tm_unseg(self.psb.tm_apid, 0, 0).unwrap();
            let tc_header = PusTmSecondaryHeader::new_simple(17, 2, &time_stamp);
            let ping_reply = PusTm::new(&mut reply_header, tc_header, None, true);
            let addr = self.psb.tm_store.add_pus_tm(&ping_reply);
            if let Err(e) = self
                .psb
                .tm_tx
                .send(addr)
                .map_err(|e| PartialPusHandlingError::TmSendError(format!("{e}")))
            {
                partial_error = Some(e);
            }
            if let Some(start_token) = start_token {
                if self
                    .psb
                    .verification_handler
                    .get_mut()
                    .completion_success(start_token, Some(&time_stamp))
                    .is_err()
                {
                    partial_error = Some(PartialPusHandlingError::VerificationError)
                }
            }
            if let Some(partial_error) = partial_error {
                return Ok(PusPacketHandlerResult::RequestHandledPartialSuccess(
                    partial_error,
                ));
            };
            return Ok(PusPacketHandlerResult::RequestHandled);
        }
        Ok(PusPacketHandlerResult::CustomSubservice(
            tc.subservice(),
            token,
        ))
    }
}
