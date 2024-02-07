use crate::tmtc::MpscStoreAndSendError;
use log::warn;
use satrs_core::pus::verification::{FailParams, StdVerifReporterWithSender};
use satrs_core::pus::{EcssTcAndToken, PusPacketHandlerResult, TcInMemory};
use satrs_core::spacepackets::ecss::tc::PusTcReader;
use satrs_core::spacepackets::ecss::PusServiceId;
use satrs_core::spacepackets::time::cds::TimeProvider;
use satrs_core::spacepackets::time::TimeWriter;
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
            .acceptance_success(init_token, Some(self.stamp_helper.stamp()))
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
                            Some(self.stamp_helper.stamp()),
                            &tmtc_err::PUS_SERVICE_NOT_IMPLEMENTED,
                            Some(&[standard_service as u8]),
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
                                Some(self.stamp_helper.stamp()),
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                Some(&[e.number]),
                            ),
                        )
                        .expect("Start failure verification failed")
                }
            }
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}
