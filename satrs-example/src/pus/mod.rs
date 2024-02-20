use crate::tmtc::MpscStoreAndSendError;
use log::warn;
use satrs::pus::verification::{
    FailParams, StdVerifReporterWithSender, VerificationReportingProvider,
};
use satrs::pus::{
    EcssTcAndToken, GenericRoutingError, PusPacketHandlerResult, PusRoutingErrorHandler, TcInMemory,
};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusServiceId;
use satrs::spacepackets::time::cds::TimeProvider;
use satrs::spacepackets::time::TimeWriter;
use satrs_example::config::{tmtc_err, CustomPusServiceId};
use std::sync::mpsc::Sender;

pub mod action;
pub mod event;
pub mod hk;
pub mod scheduler;
pub mod stack;
pub mod test;

pub struct PusTcMpscRouter {
    pub test_service_receiver: Sender<EcssTcAndToken>,
    pub event_service_receiver: Sender<EcssTcAndToken>,
    pub sched_service_receiver: Sender<EcssTcAndToken>,
    pub hk_service_receiver: Sender<EcssTcAndToken>,
    pub action_service_receiver: Sender<EcssTcAndToken>,
}

pub struct PusReceiver {
    pub verif_reporter: StdVerifReporterWithSender,
    pub pus_router: PusTcMpscRouter,
    stamp_helper: TimeStampHelper,
}

struct TimeStampHelper {
    stamper: TimeProvider,
    time_stamp: [u8; 7],
}

impl TimeStampHelper {
    pub fn new() -> Self {
        Self {
            stamper: TimeProvider::new_with_u16_days(0, 0),
            time_stamp: [0; 7],
        }
    }

    pub fn stamp(&self) -> &[u8] {
        &self.time_stamp
    }

    pub fn update_from_now(&mut self) {
        self.stamper
            .update_from_now()
            .expect("Updating timestamp failed");
        self.stamper
            .write_to_bytes(&mut self.time_stamp)
            .expect("Writing timestamp failed");
    }
}

impl PusReceiver {
    pub fn new(verif_reporter: StdVerifReporterWithSender, pus_router: PusTcMpscRouter) -> Self {
        Self {
            verif_reporter,
            pus_router,
            stamp_helper: TimeStampHelper::new(),
        }
    }
}

impl PusReceiver {
    pub fn handle_tc_packet(
        &mut self,
        tc_in_memory: TcInMemory,
        service: u8,
        pus_tc: &PusTcReader,
    ) -> Result<PusPacketHandlerResult, MpscStoreAndSendError> {
        let init_token = self.verif_reporter.add_tc(pus_tc);
        self.stamp_helper.update_from_now();
        let accepted_token = self
            .verif_reporter
            .acceptance_success(init_token, self.stamp_helper.stamp())
            .expect("Acceptance success failure");
        let service = PusServiceId::try_from(service);
        match service {
            Ok(standard_service) => match standard_service {
                PusServiceId::Test => {
                    self.pus_router.test_service_receiver.send(EcssTcAndToken {
                        tc_in_memory,
                        token: Some(accepted_token.into()),
                    })?
                }
                PusServiceId::Housekeeping => {
                    self.pus_router.hk_service_receiver.send(EcssTcAndToken {
                        tc_in_memory,
                        token: Some(accepted_token.into()),
                    })?
                }
                PusServiceId::Event => {
                    self.pus_router
                        .event_service_receiver
                        .send(EcssTcAndToken {
                            tc_in_memory,
                            token: Some(accepted_token.into()),
                        })?
                }
                PusServiceId::Scheduling => {
                    self.pus_router
                        .sched_service_receiver
                        .send(EcssTcAndToken {
                            tc_in_memory,
                            token: Some(accepted_token.into()),
                        })?
                }
                _ => {
                    let result = self.verif_reporter.start_failure(
                        accepted_token,
                        FailParams::new(
                            self.stamp_helper.stamp(),
                            &tmtc_err::PUS_SERVICE_NOT_IMPLEMENTED,
                            &[standard_service as u8],
                        ),
                    );
                    if result.is_err() {
                        warn!("Sending verification failure failed");
                    }
                }
            },
            Err(e) => {
                if let Ok(custom_service) = CustomPusServiceId::try_from(e.number) {
                    match custom_service {
                        CustomPusServiceId::Mode => {
                            // TODO: Fix mode service.
                            //self.handle_mode_service(pus_tc, accepted_token)
                        }
                        CustomPusServiceId::Health => {}
                    }
                } else {
                    self.verif_reporter
                        .start_failure(
                            accepted_token,
                            FailParams::new(
                                self.stamp_helper.stamp(),
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                &[e.number],
                            ),
                        )
                        .expect("Start failure verification failed")
                }
            }
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}

#[derive(Default)]
pub struct GenericRoutingErrorHandler<const SERVICE_ID: u8> {}

impl<const SERVICE_ID: u8> PusRoutingErrorHandler for GenericRoutingErrorHandler<SERVICE_ID> {
    type Error = satrs::pus::GenericRoutingError;

    fn handle_error(
        &self,
        target_id: satrs::TargetId,
        token: satrs::pus::verification::VerificationToken<
            satrs::pus::verification::TcStateAccepted,
        >,
        _tc: &PusTcReader,
        error: Self::Error,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) {
        warn!("Routing request for service {SERVICE_ID} failed: {error:?}");
        match error {
            GenericRoutingError::UnknownTargetId(id) => {
                let mut fail_data: [u8; 8] = [0; 8];
                fail_data.copy_from_slice(&id.to_be_bytes());
                verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(time_stamp, &tmtc_err::UNKNOWN_TARGET_ID, &fail_data),
                    )
                    .expect("Sending start failure failed");
            }
            GenericRoutingError::SendError(_) => {
                let mut fail_data: [u8; 8] = [0; 8];
                fail_data.copy_from_slice(&target_id.to_be_bytes());
                verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(time_stamp, &tmtc_err::ROUTING_ERROR, &fail_data),
                    )
                    .expect("Sending start failure failed");
            }
            GenericRoutingError::NotEnoughAppData { expected, found } => {
                let mut context_info = (found as u32).to_be_bytes().to_vec();
                context_info.extend_from_slice(&(expected as u32).to_be_bytes());
                verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(time_stamp, &tmtc_err::NOT_ENOUGH_APP_DATA, &context_info),
                    )
                    .expect("Sending start failure failed");
            }
        }
    }
}
