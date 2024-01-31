use crate::pool::SharedPool;
use crate::pus::scheduler::PusScheduler;
use crate::pus::{EcssTcReceiver, EcssTmSender, PusPacketHandlerResult, PusPacketHandlingError};
use spacepackets::ecss::{scheduling, PusPacket};
use spacepackets::time::cds::TimeProvider;
use std::boxed::Box;

use super::verification::VerificationReporterWithSender;
use super::{EcssTcInMemConverter, PusServiceBase, PusServiceHandler};

/// This is a helper class for [std] environments to handle generic PUS 11 (scheduling service)
/// packets. This handler is constrained to using the [PusScheduler], but is able to process
/// the most important PUS requests for a scheduling service.
///
/// Please note that this class does not do the regular periodic handling like releasing any
/// telecommands inside the scheduler. The user can retrieve the wrapped scheduler via the
/// [Self::scheduler] and [Self::scheduler_mut] function and then use the scheduler API to release
/// telecommands when applicable.
pub struct PusService11SchedHandler<TcInMemConverter: EcssTcInMemConverter> {
    pub psb: PusServiceHandler<TcInMemConverter>,
    shared_tc_store: SharedPool,
    scheduler: PusScheduler,
}

impl<TcInMemConverter: EcssTcInMemConverter> PusService11SchedHandler<TcInMemConverter> {
    pub fn new(
        tc_receiver: Box<dyn EcssTcReceiver>,
        tm_sender: Box<dyn EcssTmSender>,
        tm_apid: u16,
        verification_handler: VerificationReporterWithSender,
        tc_in_mem_converter: TcInMemConverter,
        shared_tc_store: SharedPool,
        scheduler: PusScheduler,
    ) -> Self {
        Self {
            psb: PusServiceHandler::new(
                tc_receiver,
                tm_sender,
                tm_apid,
                verification_handler,
                tc_in_mem_converter,
            ),
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

    pub fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        let possible_packet = self.psb.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(PusPacketHandlerResult::Empty);
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        let tc = self
            .psb
            .tc_in_mem_converter
            .convert_ecss_tc_in_memory_to_reader(&ecss_tc_and_token)?;
        let subservice = tc.subservice();
        let std_service = scheduling::Subservice::try_from(subservice);
        if std_service.is_err() {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                ecss_tc_and_token.token,
            ));
        }
        let mut partial_error = None;
        let time_stamp = PusServiceBase::get_current_timestamp(&mut partial_error);
        match std_service.unwrap() {
            scheduling::Subservice::TcEnableScheduling => {
                let start_token = self
                    .psb
                    .common
                    .verification_handler
                    .get_mut()
                    .start_success(ecss_tc_and_token.token, Some(&time_stamp))
                    .expect("Error sending start success");

                self.scheduler.enable();
                if self.scheduler.is_enabled() {
                    self.psb
                        .common
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
                    .common
                    .verification_handler
                    .get_mut()
                    .start_success(ecss_tc_and_token.token, Some(&time_stamp))
                    .expect("Error sending start success");

                self.scheduler.disable();
                if !self.scheduler.is_enabled() {
                    self.psb
                        .common
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
                    .common
                    .verification_handler
                    .get_mut()
                    .start_success(ecss_tc_and_token.token, Some(&time_stamp))
                    .expect("Error sending start success");

                let mut pool = self.shared_tc_store.write().expect("Locking pool failed");

                self.scheduler
                    .reset(pool.as_mut())
                    .expect("Error resetting TC Pool");

                self.psb
                    .common
                    .verification_handler
                    .get_mut()
                    .completion_success(start_token, Some(&time_stamp))
                    .expect("Error sending completion success");
            }
            scheduling::Subservice::TcInsertActivity => {
                let start_token = self
                    .psb
                    .common
                    .verification_handler
                    .get_mut()
                    .start_success(ecss_tc_and_token.token, Some(&time_stamp))
                    .expect("error sending start success");

                let mut pool = self.shared_tc_store.write().expect("locking pool failed");
                self.scheduler
                    .insert_wrapped_tc::<TimeProvider>(&tc, pool.as_mut())
                    .expect("insertion of activity into pool failed");

                self.psb
                    .common
                    .verification_handler
                    .get_mut()
                    .completion_success(start_token, Some(&time_stamp))
                    .expect("sending completion success failed");
            }
            _ => {
                // Treat unhandled standard subservices as custom subservices for now.
                return Ok(PusPacketHandlerResult::CustomSubservice(
                    tc.subservice(),
                    ecss_tc_and_token.token,
                ));
            }
        }
        if let Some(partial_error) = partial_error {
            return Ok(PusPacketHandlerResult::RequestHandledPartialSuccess(
                partial_error,
            ));
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}
