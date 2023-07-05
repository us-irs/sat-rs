use crate::pool::{SharedPool, StoreAddr};
use crate::pus::scheduler::PusScheduler;
use crate::pus::verification::{StdVerifReporterWithSender, TcStateAccepted, VerificationToken};
use crate::pus::{
    AcceptedTc, PusPacketHandlerResult, PusPacketHandlingError, PusServiceBase, PusServiceHandler,
};
use crate::tmtc::tm_helper::SharedTmStore;
use spacepackets::ecss::{scheduling, PusPacket};
use spacepackets::tc::PusTc;
use spacepackets::time::cds::TimeProvider;
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
        self.copy_tc_to_buf(addr)?;
        let (tc, _) = PusTc::from_bytes(&self.psb.pus_buf).unwrap();
        let std_service = scheduling::Subservice::try_from(tc.subservice());
        if std_service.is_err() {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                token,
            ));
        }
        let mut partial_error = None;
        let time_stamp = self.psb().get_current_timestamp(&mut partial_error);
        match std_service.unwrap() {
            scheduling::Subservice::TcEnableScheduling => {
                let start_token = self
                    .psb
                    .verification_handler
                    .start_success(token, Some(&time_stamp))
                    .expect("Error sending start success");

                self.scheduler.enable();
                if self.scheduler.is_enabled() {
                    self.psb
                        .verification_handler
                        .completion_success(start_token, Some(&time_stamp))
                        .expect("Error sending completion success");
                } else {
                    panic!("Failed to enable scheduler");
                }
            }
            scheduling::Subservice::TcDisableScheduling => {
                let start_token = self
                    .psb
                    .verification_handler
                    .start_success(token, Some(&time_stamp))
                    .expect("Error sending start success");

                self.scheduler.disable();
                if !self.scheduler.is_enabled() {
                    self.psb
                        .verification_handler
                        .completion_success(start_token, Some(&time_stamp))
                        .expect("Error sending completion success");
                } else {
                    panic!("Failed to disable scheduler");
                }
            }
            scheduling::Subservice::TcResetScheduling => {
                let start_token = self
                    .psb
                    .verification_handler
                    .start_success(token, Some(&time_stamp))
                    .expect("Error sending start success");

                let mut pool = self.psb.tc_store.write().expect("Locking pool failed");

                self.scheduler
                    .reset(pool.as_mut())
                    .expect("Error resetting TC Pool");

                self.psb
                    .verification_handler
                    .completion_success(start_token, Some(&time_stamp))
                    .expect("Error sending completion success");
            }
            scheduling::Subservice::TcInsertActivity => {
                let start_token = self
                    .psb
                    .verification_handler
                    .start_success(token, Some(&time_stamp))
                    .expect("error sending start success");

                let mut pool = self.psb.tc_store.write().expect("locking pool failed");
                self.scheduler
                    .insert_wrapped_tc::<TimeProvider>(&tc, pool.as_mut())
                    .expect("insertion of activity into pool failed");

                self.psb
                    .verification_handler
                    .completion_success(start_token, Some(&time_stamp))
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
