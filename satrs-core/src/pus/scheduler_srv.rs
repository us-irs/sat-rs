use crate::pool::{SharedPool, StoreAddr};
use crate::pus::scheduler::PusScheduler;
use crate::pus::verification::{StdVerifReporterWithSender, TcStateAccepted, VerificationToken};
use crate::pus::{
    AcceptedTc, PartialPusHandlingError, PusPacketHandlerResult, PusPacketHandlingError,
    PusServiceBase, PusServiceHandler,
};
use crate::tmtc::tm_helper::SharedTmStore;
use spacepackets::ecss::{scheduling, PusPacket};
use spacepackets::tc::PusTc;
use spacepackets::time::cds::TimeProvider;
use spacepackets::time::TimeWriter;
use std::format;
use std::sync::mpsc::{Receiver, Sender};

pub struct PusService11SchedHandler {
    psb: PusServiceBase,
    scheduler: PusScheduler,
}

impl PusService11SchedHandler {
    pub fn new(
        receiver: Receiver<AcceptedTc>,
        tc_pool: SharedPool,
        tm_tx: Sender<StoreAddr>,
        tm_store: SharedTmStore,
        tm_apid: u16,
        verification_handler: StdVerifReporterWithSender,
        scheduler: PusScheduler,
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
            scheduler,
        }
    }

    pub fn scheduler_mut(&mut self) -> &mut PusScheduler {
        &mut self.scheduler
    }
}

impl PusServiceHandler for PusService11SchedHandler {
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
        let std_service = scheduling::Subservice::try_from(tc.subservice());
        if std_service.is_err() {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                token,
            ));
        }
        //let partial_error = self.psb.update_stamp().err();
        let time_provider =
            TimeProvider::from_now_with_u16_days().map_err(PartialPusHandlingError::TimeError);
        let partial_error = if let Ok(time_provider) = time_provider {
            time_provider
                .write_to_bytes(&mut self.psb.stamp_buf)
                .unwrap();
            Ok(())
        } else {
            self.psb.stamp_buf = [0; 7];
            Err(time_provider.unwrap_err())
        };
        let partial_error = partial_error.err();
        match std_service.unwrap() {
            scheduling::Subservice::TcEnableScheduling => {
                let start_token = self
                    .psb
                    .verification_handler
                    .start_success(token, Some(&self.psb.stamp_buf))
                    .expect("Error sending start success");

                self.scheduler.enable();
                if self.scheduler.is_enabled() {
                    self.psb
                        .verification_handler
                        .completion_success(start_token, Some(&self.psb.stamp_buf))
                        .expect("Error sending completion success");
                } else {
                    panic!("Failed to enable scheduler");
                }
            }
            scheduling::Subservice::TcDisableScheduling => {
                let start_token = self
                    .psb
                    .verification_handler
                    .start_success(token, Some(&self.psb.stamp_buf))
                    .expect("Error sending start success");

                self.scheduler.disable();
                if !self.scheduler.is_enabled() {
                    self.psb
                        .verification_handler
                        .completion_success(start_token, Some(&self.psb.stamp_buf))
                        .expect("Error sending completion success");
                } else {
                    panic!("Failed to disable scheduler");
                }
            }
            scheduling::Subservice::TcResetScheduling => {
                let start_token = self
                    .psb
                    .verification_handler
                    .start_success(token, Some(&self.psb.stamp_buf))
                    .expect("Error sending start success");

                let mut pool = self.psb.tc_store.write().expect("Locking pool failed");

                self.scheduler
                    .reset(pool.as_mut())
                    .expect("Error resetting TC Pool");

                self.psb
                    .verification_handler
                    .completion_success(start_token, Some(&self.psb.stamp_buf))
                    .expect("Error sending completion success");
            }
            scheduling::Subservice::TcInsertActivity => {
                let start_token = self
                    .psb
                    .verification_handler
                    .start_success(token, Some(&self.psb.stamp_buf))
                    .expect("error sending start success");

                let mut pool = self.psb.tc_store.write().expect("locking pool failed");
                self.scheduler
                    .insert_wrapped_tc::<TimeProvider>(&tc, pool.as_mut())
                    .expect("insertion of activity into pool failed");

                self.psb
                    .verification_handler
                    .completion_success(start_token, Some(&self.psb.stamp_buf))
                    .expect("sending completion success failed");
            }
            _ => {
                return Ok(PusPacketHandlerResult::CustomSubservice(
                    tc.subservice(),
                    token,
                ));
            }
        }
        if let Some(partial_error) = partial_error {
            return Ok(PusPacketHandlerResult::RequestHandledPartialSuccess(
                partial_error,
            ));
        }
        Ok(PusPacketHandlerResult::CustomSubservice(
            tc.subservice(),
            token,
        ))
    }
}
