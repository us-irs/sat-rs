use crate::pus::{AcceptedTc, PusPacketHandlerResult, PusPacketHandlingError, PusServiceBase};
use delegate::delegate;
use log::{error, info, warn};
use satrs_core::events::EventU32;
use satrs_core::params::Params;
use satrs_core::pool::{SharedPool, StoreAddr, StoreError};
use satrs_core::pus::verification::{
    FailParams, StdVerifReporterWithSender, TcStateAccepted, TcStateStarted,
    VerificationOrSendErrorWithToken, VerificationToken,
};
use satrs_core::seq_count::{SeqCountProviderSyncClonable, SequenceCountProviderCore};
use satrs_core::spacepackets::ecss::{PusError, PusPacket};
use satrs_core::spacepackets::tc::PusTc;
use satrs_core::spacepackets::time::cds::TimeProvider;
use satrs_core::spacepackets::time::{StdTimestampError, TimeWriter};
use satrs_core::spacepackets::tm::PusTm;
use satrs_core::tmtc::tm_helper::{PusTmWithCdsShortHelper, SharedTmStore};
use satrs_example::{tmtc_err, TEST_EVENT};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

pub struct Service17CustomWrapper {
    pub pus17_handler: PusService17TestHandler,
    pub test_srv_event_sender: Sender<(EventU32, Option<Params>)>,
}

impl Service17CustomWrapper {
    pub fn perform_operation(&mut self) -> bool {
        let mut handled_pings = 0;
        let res = self.pus17_handler.handle_next_packet();
        if res.is_err() {
            warn!("PUS17 handler failed with error {:?}", res.unwrap_err());
            return true;
        }
        match res.unwrap() {
            PusPacketHandlerResult::RequestHandled => {
                info!("Received PUS ping command TC[17,1]");
                info!("Sent ping reply PUS TM[17,2]");
                handled_pings += 1;
            }
            PusPacketHandlerResult::CustomSubservice(token) => {
                let (buf, _) = self.pus17_handler.pus_tc_buf();
                let (tc, size) = PusTc::from_bytes(buf).unwrap();
                let time_stamper = TimeProvider::from_now_with_u16_days().unwrap();
                let mut stamp_buf: [u8; 7] = [0; 7];
                time_stamper.write_to_bytes(&mut stamp_buf).unwrap();
                if tc.subservice() == 128 {
                    info!("Generating test event");
                    self.test_srv_event_sender
                        .send((TEST_EVENT.into(), None))
                        .expect("Sending test event failed");
                    let start_token = self
                        .pus17_handler
                        .verification_handler()
                        .start_success(token, Some(&stamp_buf))
                        .expect("Error sending start success");
                    self.pus17_handler
                        .verification_handler()
                        .completion_success(start_token, Some(&stamp_buf))
                        .expect("Error sending completion success");
                } else {
                    let fail_data = [tc.subservice()];
                    self.pus17_handler
                        .verification_handler()
                        .start_failure(
                            token,
                            FailParams::new(
                                Some(&stamp_buf),
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                Some(&fail_data),
                            ),
                        )
                        .expect("Sending start failure verification failed");
                }
            }
            PusPacketHandlerResult::Empty => {
                return false;
            }
        }
        true
    }
}

pub struct PusService17TestHandler {
    psb: PusServiceBase,
}

impl PusService17TestHandler {
    pub fn new(
        receiver: Receiver<AcceptedTc>,
        tc_pool: SharedPool,
        tm_helper: PusTmWithCdsShortHelper,
        tm_tx: Sender<StoreAddr>,
        tm_store: SharedTmStore,
        verification_handler: StdVerifReporterWithSender,
    ) -> Self {
        Self {
            psb: PusServiceBase::new(
                receiver,
                tc_pool,
                tm_helper,
                tm_tx,
                tm_store,
                verification_handler,
            ),
        }
    }

    pub fn verification_handler(&mut self) -> &mut StdVerifReporterWithSender {
        &mut self.psb.verification_handler
    }

    pub fn pus_tc_buf(&self) -> (&[u8], usize) {
        (&self.psb.pus_buf, self.psb.pus_size)
    }

    pub fn handle_next_packet(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        return match self.psb.tc_rx.try_recv() {
            Ok((addr, token)) => {
                if self.handle_one_tc(addr, token)? {
                    return Ok(PusPacketHandlerResult::RequestHandled);
                }
                Ok(PusPacketHandlerResult::CustomSubservice(token))
            }
            Err(e) => match e {
                TryRecvError::Empty => Ok(PusPacketHandlerResult::Empty),
                TryRecvError::Disconnected => Err(PusPacketHandlingError::QueueDisconnected),
            },
        };
    }

    pub fn handle_one_tc(
        &mut self,
        addr: StoreAddr,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<bool, PusPacketHandlingError> {
        {
            // Keep locked section as short as possible.
            let mut tc_pool = self
                .psb
                .tc_store
                .write()
                .map_err(|e| PusPacketHandlingError::RwGuardError(format!("{e}")))?;
            let tc_guard = tc_pool.read_with_guard(addr);
            let tc_raw = tc_guard.read().expect("Reading pool guard failed");
            self.psb.pus_buf[0..tc_raw.len()].copy_from_slice(tc_raw);
        }
        let (tc, tc_size) = PusTc::from_bytes(&self.psb.pus_buf)?;
        if tc.service() != 17 {
            return Err(PusPacketHandlingError::WrongService(tc.service()));
        }
        if tc.subservice() == 1 {
            let time_provider = TimeProvider::from_now_with_u16_days()?;
            // Can not fail, buffer is large enough.
            time_provider
                .write_to_bytes(&mut self.psb.stamp_buf)
                .unwrap();
            let result = self
                .psb
                .verification_handler
                .start_success(token, Some(&self.psb.stamp_buf));
            let start_token = if result.is_err() {
                error!("Could not send start success verification");
                None
            } else {
                Some(result.unwrap())
            };
            // Sequence count will be handled centrally in TM funnel.
            let ping_reply =
                self.psb
                    .tm_helper
                    .create_pus_tm_with_stamp(17, 2, None, &time_provider, 0);
            let addr = self.psb.tm_store.add_pus_tm(&ping_reply);
            self.psb
                .tm_tx
                .send(addr)
                .map_err(|e| PusPacketHandlingError::TmSendError(format!("{e}")))?;
            if let Some(start_token) = start_token {
                if self
                    .psb
                    .verification_handler
                    .completion_success(start_token, Some(&self.psb.stamp_buf))
                    .is_err()
                {
                    error!("Could not send completion success verification");
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}
