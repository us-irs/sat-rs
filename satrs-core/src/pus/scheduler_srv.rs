use crate::pool::{PoolGuard, SharedPool, StoreAddr};
use crate::pus::scheduler::PusScheduler;
use crate::pus::verification::{StdVerifReporterWithSender, TcStateAccepted, VerificationToken};
use crate::pus::{
    AcceptedTc, EcssTcReceiver, EcssTmSender, PusPacketHandlerResult, PusPacketHandlingError,
    PusServiceBase, PusServiceHandler, ReceivedTcWrapper,
};
use spacepackets::ecss::{scheduling, PusPacket};
use spacepackets::tc::PusTc;
use spacepackets::time::cds::TimeProvider;
use std::boxed::Box;
use std::sync::mpsc::Receiver;

/// This is a helper class for [std] environments to handle generic PUS 11 (scheduling service)
/// packets. This handler is constrained to using the [PusScheduler], but is able to process
/// the most important PUS requests for a scheduling service.
///
/// Please note that this class does not do the regular periodic handling like releasing any
/// telecommands inside the scheduler. The user can retrieve the wrapped scheduler via the
/// [Self::scheduler] and [Self::scheduler_mut] function and then use the scheduler API to release
/// telecommands when applicable.
pub struct PusService11SchedHandler {
    psb: PusServiceBase,
    shared_tc_store: SharedPool,
    scheduler: PusScheduler,
}

impl PusService11SchedHandler {
    pub fn new(
        tc_receiver: Box<dyn EcssTcReceiver>,
        tm_sender: Box<dyn EcssTmSender>,
        tm_apid: u16,
        verification_handler: StdVerifReporterWithSender,
        shared_tc_store: SharedPool,
        scheduler: PusScheduler,
    ) -> Self {
        Self {
            psb: PusServiceBase::new(tc_receiver, tm_sender, tm_apid, verification_handler),
            shared_tc_store,
            scheduler,
        }
    }

    pub fn scheduler_mut(&mut self) -> &mut PusScheduler {
        &mut self.scheduler
    }

    pub fn scheduler(&self) -> &PusScheduler {
        &self.scheduler
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
        tc: PusTc,
        tc_guard: PoolGuard,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        let std_service = scheduling::Subservice::try_from(tc.subservice());
        if std_service.is_err() {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                token.into(),
            ));
        }
        let mut partial_error = None;
        let time_stamp = self.psb().get_current_timestamp(&mut partial_error);
        match std_service.unwrap() {
            scheduling::Subservice::TcEnableScheduling => {
                let start_token = self
                    .psb
                    .verification_handler
                    .get_mut()
                    .start_success(token, Some(&time_stamp))
                    .expect("Error sending start success");

                self.scheduler.enable();
                if self.scheduler.is_enabled() {
                    self.psb
                        .verification_handler
                        .get_mut()
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
                    .get_mut()
                    .start_success(token, Some(&time_stamp))
                    .expect("Error sending start success");

                self.scheduler.disable();
                if !self.scheduler.is_enabled() {
                    self.psb
                        .verification_handler
                        .get_mut()
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
                    .get_mut()
                    .start_success(token, Some(&time_stamp))
                    .expect("Error sending start success");

                let mut pool = self.shared_tc_store.write().expect("Locking pool failed");

                self.scheduler
                    .reset(pool.as_mut())
                    .expect("Error resetting TC Pool");

                self.psb
                    .verification_handler
                    .get_mut()
                    .completion_success(start_token, Some(&time_stamp))
                    .expect("Error sending completion success");
            }
            scheduling::Subservice::TcInsertActivity => {
                let start_token = self
                    .psb
                    .verification_handler
                    .get_mut()
                    .start_success(token, Some(&time_stamp))
                    .expect("error sending start success");

                let mut pool = self.shared_tc_store.write().expect("locking pool failed");
                self.scheduler
                    .insert_wrapped_tc::<TimeProvider>(&tc, pool.as_mut())
                    .expect("insertion of activity into pool failed");

                self.psb
                    .verification_handler
                    .get_mut()
                    .completion_success(start_token, Some(&time_stamp))
                    .expect("sending completion success failed");
            }
            _ => {
                return Ok(PusPacketHandlerResult::CustomSubservice(
                    tc.subservice(),
                    token.into(),
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
            token.into(),
        ))
    }
}
