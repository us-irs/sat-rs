use crate::requests::{Request, RequestWithToken};
use crate::tmtc::{PusTcSource, TmStore};
use log::{info, warn};
use satrs_core::events::EventU32;
use satrs_core::hk::{CollectionIntervalFactor, HkRequest};
use satrs_core::mode::{ModeAndSubmode, ModeRequest};
use satrs_core::params::Params;
use satrs_core::pool::{PoolProvider, StoreAddr};
use satrs_core::pus::event_man::{EventRequest, EventRequestWithToken};
use satrs_core::pus::hk;
use satrs_core::pus::mode;
use satrs_core::pus::mode::Subservice;
use satrs_core::pus::scheduling::PusScheduler;
use satrs_core::pus::verification::{
    pus_11_generic_tc_check, FailParams, StdVerifReporterWithSender, TcStateAccepted,
    VerificationToken,
};
use satrs_core::pus::{event, GenericTcCheckError};
use satrs_core::res_code::ResultU16;
use satrs_core::seq_count::{SeqCountProviderSyncClonable, SequenceCountProviderCore};
use satrs_core::spacepackets::ecss::{scheduling, PusServiceId};
use satrs_core::spacepackets::time::CcsdsTimeProvider;
use satrs_core::tmtc::tm_helper::PusTmWithCdsShortHelper;
use satrs_core::tmtc::{AddressableId, PusServiceProvider, TargetId};
use satrs_core::{
    spacepackets::ecss::PusPacket, spacepackets::tc::PusTc, spacepackets::time::cds::TimeProvider,
    spacepackets::time::TimeWriter, spacepackets::SpHeader,
};
use satrs_example::{hk_err, tmtc_err, CustomPusServiceId, TEST_EVENT};
use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};

pub struct PusReceiver {
    pub tm_helper: PusTmWithCdsShortHelper,
    pub tm_args: PusTmArgs,
    pub tc_args: PusTcArgs,
    stamp_helper: TimeStampHelper,
}

pub struct PusTmArgs {
    /// All telemetry is sent with this sender handle.
    pub tm_tx: Sender<StoreAddr>,
    /// All TM to be sent is stored here
    pub tm_store: TmStore,
    /// All verification reporting is done with this reporter.
    pub verif_reporter: StdVerifReporterWithSender,
    /// Sequence count provider for TMs sent from within pus demultiplexer
    pub seq_count_provider: SeqCountProviderSyncClonable,
}

impl PusTmArgs {
    fn vr(&mut self) -> &mut StdVerifReporterWithSender {
        &mut self.verif_reporter
    }
}

#[allow(dead_code)]
pub struct PusTcHandlerBase {
    pub tc_store: Box<dyn PoolProvider>,
    pub receiver: Receiver<(StoreAddr, VerificationToken<TcStateAccepted>)>,
    pub verif_reporter: StdVerifReporterWithSender,
    pub time_provider: Box<dyn CcsdsTimeProvider>,
}

pub trait TestHandlerNoPing {
    fn handle_no_ping_tc(&mut self, tc: PusTc);
}

#[allow(dead_code)]
pub struct PusTestTcHandler {
    pub base: PusTcHandlerBase,
    handler: Option<Box<dyn TestHandlerNoPing>>,
}

#[allow(dead_code)]
pub struct PusScheduleTcHandler {
    pub base: PusTestTcHandler,
}

impl PusTestTcHandler {
    #[allow(dead_code)]
    pub fn operation(&mut self) {
        let (addr, token) = self.base.receiver.recv().unwrap();
        let data = self.base.tc_store.read(&addr).unwrap();
        let (pus_tc, _len) = PusTc::from_bytes(data).unwrap();
        let stamp: [u8; 7] = [0; 7];
        if pus_tc.subservice() == 1 {
            self.base
                .verif_reporter
                .completion_success(token, Some(&stamp))
                .unwrap();
        } else if let Some(handler) = &mut self.handler {
            handler.handle_no_ping_tc(pus_tc);
        }
    }
}

pub struct PusTcArgs {
    pub event_request_tx: Sender<EventRequestWithToken>,
    /// Request routing helper. Maps targeted requests to their recipient.
    pub request_map: HashMap<TargetId, Sender<RequestWithToken>>,
    /// Required for scheduling of telecommands.
    pub tc_source: PusTcSource,
    pub event_sender: Sender<(EventU32, Option<Params>)>,
    pub scheduler: Rc<RefCell<PusScheduler>>,
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
    pub fn new(apid: u16, tm_arguments: PusTmArgs, tc_arguments: PusTcArgs) -> Self {
        Self {
            tm_helper: PusTmWithCdsShortHelper::new(apid),
            tm_args: tm_arguments,
            tc_args: tc_arguments,
            stamp_helper: TimeStampHelper::new(),
        }
    }
}

impl PusServiceProvider for PusReceiver {
    type Error = ();

    fn handle_pus_tc_packet(
        &mut self,
        service: u8,
        _header: &SpHeader,
        pus_tc: &PusTc,
    ) -> Result<(), Self::Error> {
        let init_token = self.tm_args.verif_reporter.add_tc(pus_tc);
        self.stamp_helper.update_from_now();
        let accepted_token = self
            .tm_args
            .vr()
            .acceptance_success(init_token, Some(self.stamp_helper.stamp()))
            .expect("Acceptance success failure");
        let service = PusServiceId::try_from(service);
        match service {
            Ok(standard_service) => match standard_service {
                PusServiceId::Test => self.handle_test_service(pus_tc, accepted_token),
                PusServiceId::Housekeeping => self.handle_hk_request(pus_tc, accepted_token),
                PusServiceId::Event => self.handle_event_request(pus_tc, accepted_token),
                PusServiceId::Scheduling => self.handle_scheduled_tc(pus_tc, accepted_token),
                _ => self
                    .tm_args
                    .verif_reporter
                    .start_failure(
                        accepted_token,
                        FailParams::new(
                            Some(self.stamp_helper.stamp()),
                            &tmtc_err::PUS_SERVICE_NOT_IMPLEMENTED,
                            Some(&[standard_service as u8]),
                        ),
                    )
                    .expect("Start failure verification failed"),
            },
            Err(e) => {
                if let Ok(custom_service) = CustomPusServiceId::try_from(e.number) {
                    match custom_service {
                        CustomPusServiceId::Mode => {
                            self.handle_mode_service(pus_tc, accepted_token)
                        }
                        CustomPusServiceId::Health => {}
                    }
                } else {
                    self.tm_args
                        .verif_reporter
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
        Ok(())
    }
}

impl PusReceiver {
    fn handle_test_service(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        match PusPacket::subservice(pus_tc) {
            1 => {
                info!("Received PUS ping command TC[17,1]");
                info!("Sending ping reply PUS TM[17,2]");
                let start_token = self
                    .tm_args
                    .verif_reporter
                    .start_success(token, Some(self.stamp_helper.stamp()))
                    .expect("Error sending start success");
                let ping_reply = self.tm_helper.create_pus_tm_timestamp_now(
                    17,
                    2,
                    None,
                    self.tm_args.seq_count_provider.get(),
                );
                let addr = self.tm_args.tm_store.add_pus_tm(&ping_reply);
                self.tm_args
                    .tm_tx
                    .send(addr)
                    .expect("Sending TM to TM funnel failed");
                self.tm_args.seq_count_provider.increment();
                self.tm_args
                    .verif_reporter
                    .completion_success(start_token, Some(self.stamp_helper.stamp()))
                    .expect("Error sending completion success");
            }
            128 => {
                info!("Generating test event");
                self.tc_args
                    .event_sender
                    .send((TEST_EVENT.into(), None))
                    .expect("Sending test event failed");
                let start_token = self
                    .tm_args
                    .verif_reporter
                    .start_success(token, Some(self.stamp_helper.stamp()))
                    .expect("Error sending start success");
                self.tm_args
                    .verif_reporter
                    .completion_success(start_token, Some(self.stamp_helper.stamp()))
                    .expect("Error sending completion success");
            }
            _ => {
                self.tm_args
                    .verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(
                            Some(self.stamp_helper.stamp()),
                            &tmtc_err::INVALID_PUS_SUBSERVICE,
                            None,
                        ),
                    )
                    .expect("Sending start failure TM failed");
            }
        }
    }

    fn handle_hk_request(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        if pus_tc.user_data().is_none() {
            self.tm_args
                .verif_reporter
                .start_failure(
                    token,
                    FailParams::new(
                        Some(self.stamp_helper.stamp()),
                        &tmtc_err::NOT_ENOUGH_APP_DATA,
                        None,
                    ),
                )
                .expect("Sending start failure TM failed");
            return;
        }
        let user_data = pus_tc.user_data().unwrap();
        if user_data.len() < 8 {
            let err = if user_data.len() < 4 {
                &hk_err::TARGET_ID_MISSING
            } else {
                &hk_err::UNIQUE_ID_MISSING
            };
            self.tm_args
                .verif_reporter
                .start_failure(
                    token,
                    FailParams::new(Some(self.stamp_helper.stamp()), err, None),
                )
                .expect("Sending start failure TM failed");
            return;
        }
        let addressable_id = AddressableId::from_raw_be(user_data).unwrap();
        if !self
            .tc_args
            .request_map
            .contains_key(&addressable_id.target_id)
        {
            self.tm_args
                .verif_reporter
                .start_failure(
                    token,
                    FailParams::new(
                        Some(self.stamp_helper.stamp()),
                        &hk_err::UNKNOWN_TARGET_ID,
                        None,
                    ),
                )
                .expect("Sending start failure TM failed");
            return;
        }
        let send_request = |target: TargetId, request: HkRequest| {
            let sender = self
                .tc_args
                .request_map
                .get(&addressable_id.target_id)
                .unwrap();
            sender
                .send(RequestWithToken::new(
                    target,
                    Request::HkRequest(request),
                    token,
                ))
                .unwrap_or_else(|_| panic!("Sending HK request {request:?} failed"));
        };
        if PusPacket::subservice(pus_tc) == hk::Subservice::TcEnableHkGeneration as u8 {
            send_request(
                addressable_id.target_id,
                HkRequest::Enable(addressable_id.unique_id),
            );
        } else if PusPacket::subservice(pus_tc) == hk::Subservice::TcDisableHkGeneration as u8 {
            send_request(
                addressable_id.target_id,
                HkRequest::Disable(addressable_id.unique_id),
            );
        } else if PusPacket::subservice(pus_tc) == hk::Subservice::TcGenerateOneShotHk as u8 {
            send_request(
                addressable_id.target_id,
                HkRequest::OneShot(addressable_id.unique_id),
            );
        } else if PusPacket::subservice(pus_tc)
            == hk::Subservice::TcModifyHkCollectionInterval as u8
        {
            if user_data.len() < 12 {
                self.tm_args
                    .verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(
                            Some(self.stamp_helper.stamp()),
                            &hk_err::COLLECTION_INTERVAL_MISSING,
                            None,
                        ),
                    )
                    .expect("Sending start failure TM failed");
                return;
            }
            send_request(
                addressable_id.target_id,
                HkRequest::ModifyCollectionInterval(
                    addressable_id.unique_id,
                    CollectionIntervalFactor::from_be_bytes(user_data[8..12].try_into().unwrap()),
                ),
            );
        }
    }

    fn handle_event_request(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        let send_start_failure = |vr: &mut StdVerifReporterWithSender,
                                  timestamp: &[u8],
                                  failure_code: &ResultU16,
                                  failure_data: Option<&[u8]>| {
            vr.start_failure(
                token,
                FailParams::new(Some(timestamp), failure_code, failure_data),
            )
            .expect("Sending start failure TM failed");
        };
        let send_start_acceptance = |vr: &mut StdVerifReporterWithSender, timestamp: &[u8]| {
            vr.start_success(token, Some(timestamp))
                .expect("Sending start success TM failed")
        };
        if pus_tc.user_data().is_none() {
            send_start_failure(
                &mut self.tm_args.verif_reporter,
                self.stamp_helper.stamp(),
                &tmtc_err::NOT_ENOUGH_APP_DATA,
                None,
            );
            return;
        }
        let app_data = pus_tc.user_data().unwrap();
        if app_data.len() < 4 {
            send_start_failure(
                &mut self.tm_args.verif_reporter,
                self.stamp_helper.stamp(),
                &tmtc_err::NOT_ENOUGH_APP_DATA,
                None,
            );
            return;
        }
        let event_id = EventU32::from(u32::from_be_bytes(app_data.try_into().unwrap()));
        match PusPacket::subservice(pus_tc).try_into() {
            Ok(event::Subservice::TcEnableEventGeneration) => {
                let start_token = send_start_acceptance(
                    &mut self.tm_args.verif_reporter,
                    self.stamp_helper.stamp(),
                );
                self.tc_args
                    .event_request_tx
                    .send(EventRequestWithToken {
                        request: EventRequest::Enable(event_id),
                        token: start_token,
                    })
                    .expect("Sending event request failed");
            }
            Ok(event::Subservice::TcDisableEventGeneration) => {
                let start_token = send_start_acceptance(
                    &mut self.tm_args.verif_reporter,
                    self.stamp_helper.stamp(),
                );
                self.tc_args
                    .event_request_tx
                    .send(EventRequestWithToken {
                        request: EventRequest::Disable(event_id),
                        token: start_token,
                    })
                    .expect("Sending event request failed");
            }
            _ => {
                send_start_failure(
                    &mut self.tm_args.verif_reporter,
                    self.stamp_helper.stamp(),
                    &tmtc_err::INVALID_PUS_SUBSERVICE,
                    None,
                );
            }
        }
    }

    fn handle_scheduled_tc(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        let subservice = match pus_11_generic_tc_check(pus_tc) {
            Ok(subservice) => subservice,
            Err(e) => match e {
                GenericTcCheckError::NotEnoughAppData => {
                    self.tm_args
                        .verif_reporter
                        .start_failure(
                            token,
                            FailParams::new(
                                Some(self.stamp_helper.stamp()),
                                &tmtc_err::NOT_ENOUGH_APP_DATA,
                                None,
                            ),
                        )
                        .expect("could not sent verification error");
                    return;
                }
                GenericTcCheckError::InvalidSubservice => {
                    self.tm_args
                        .verif_reporter
                        .start_failure(
                            token,
                            FailParams::new(
                                Some(self.stamp_helper.stamp()),
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                None,
                            ),
                        )
                        .expect("could not sent verification error");
                    return;
                }
            },
        };
        match subservice {
            scheduling::Subservice::TcEnableScheduling => {
                let start_token = self
                    .tm_args
                    .verif_reporter
                    .start_success(token, Some(self.stamp_helper.stamp()))
                    .expect("Error sending start success");

                let mut scheduler = self.tc_args.scheduler.borrow_mut();
                scheduler.enable();
                if scheduler.is_enabled() {
                    self.tm_args
                        .verif_reporter
                        .completion_success(start_token, Some(self.stamp_helper.stamp()))
                        .expect("Error sending completion success");
                } else {
                    panic!("Failed to enable scheduler");
                }
            }
            scheduling::Subservice::TcDisableScheduling => {
                let start_token = self
                    .tm_args
                    .verif_reporter
                    .start_success(token, Some(self.stamp_helper.stamp()))
                    .expect("Error sending start success");

                let mut scheduler = self.tc_args.scheduler.borrow_mut();
                scheduler.disable();
                if !scheduler.is_enabled() {
                    self.tm_args
                        .verif_reporter
                        .completion_success(start_token, Some(self.stamp_helper.stamp()))
                        .expect("Error sending completion success");
                } else {
                    panic!("Failed to disable scheduler");
                }
            }
            scheduling::Subservice::TcResetScheduling => {
                let start_token = self
                    .tm_args
                    .verif_reporter
                    .start_success(token, Some(self.stamp_helper.stamp()))
                    .expect("Error sending start success");

                let mut pool = self
                    .tc_args
                    .tc_source
                    .tc_store
                    .pool
                    .write()
                    .expect("Locking pool failed");

                let mut scheduler = self.tc_args.scheduler.borrow_mut();
                scheduler
                    .reset(pool.as_mut())
                    .expect("Error resetting TC Pool");
                drop(scheduler);

                self.tm_args
                    .verif_reporter
                    .completion_success(start_token, Some(self.stamp_helper.stamp()))
                    .expect("Error sending completion success");
            }
            scheduling::Subservice::TcInsertActivity => {
                let start_token = self
                    .tm_args
                    .verif_reporter
                    .start_success(token, Some(self.stamp_helper.stamp()))
                    .expect("error sending start success");

                let mut pool = self
                    .tc_args
                    .tc_source
                    .tc_store
                    .pool
                    .write()
                    .expect("locking pool failed");
                let mut scheduler = self.tc_args.scheduler.borrow_mut();
                scheduler
                    .insert_wrapped_tc::<TimeProvider>(pus_tc, pool.as_mut())
                    .expect("insertion of activity into pool failed");
                drop(scheduler);

                self.tm_args
                    .verif_reporter
                    .completion_success(start_token, Some(self.stamp_helper.stamp()))
                    .expect("sending completion success failed");
            }
            _ => {}
        }
    }

    fn handle_mode_service(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        let mut app_data_len = 0;
        let app_data = pus_tc.user_data();
        if app_data.is_some() {
            app_data_len = pus_tc.user_data().unwrap().len();
        }
        if app_data_len < 4 {
            self.tm_args
                .verif_reporter
                .start_failure(
                    token,
                    FailParams::new(
                        Some(self.stamp_helper.stamp()),
                        &tmtc_err::NOT_ENOUGH_APP_DATA,
                        Some(format!("expected {} bytes, found {}", 4, app_data_len).as_bytes()),
                    ),
                )
                .expect("Sending start failure TM failed");
        }
        let app_data = app_data.unwrap();
        let mut invalid_subservice_handler = || {
            self.tm_args
                .verif_reporter
                .start_failure(
                    token,
                    FailParams::new(
                        Some(self.stamp_helper.stamp()),
                        &tmtc_err::INVALID_PUS_SUBSERVICE,
                        Some(&[PusPacket::subservice(pus_tc)]),
                    ),
                )
                .expect("Sending start failure TM failed");
        };
        let subservice = mode::Subservice::try_from(PusPacket::subservice(pus_tc));
        if let Ok(subservice) = subservice {
            let forward_mode_request = |target_id, mode_request: ModeRequest| match self
                .tc_args
                .request_map
                .get(&target_id)
            {
                None => warn!("not mode request recipient for target ID {target_id} found"),
                Some(sender_to_recipient) => {
                    sender_to_recipient
                        .send(RequestWithToken::new(
                            target_id,
                            Request::ModeRequest(mode_request),
                            token,
                        ))
                        .expect("sending mode request failed");
                }
            };
            let mut valid_subservice = true;
            match subservice {
                Subservice::TcSetMode => {
                    let target_id = u32::from_be_bytes(app_data[0..4].try_into().unwrap());
                    let min_len = ModeAndSubmode::raw_len() + 4;
                    if app_data_len < min_len {
                        self.tm_args
                            .verif_reporter
                            .start_failure(
                                token,
                                FailParams::new(
                                    Some(self.stamp_helper.stamp()),
                                    &tmtc_err::NOT_ENOUGH_APP_DATA,
                                    Some(
                                        format!("expected {min_len} bytes, found {app_data_len}")
                                            .as_bytes(),
                                    ),
                                ),
                            )
                            .expect("Sending start failure TM failed");
                    }
                    // Should never fail after size check
                    let mode_submode = ModeAndSubmode::from_be_bytes(
                        app_data[4..4 + ModeAndSubmode::raw_len()]
                            .try_into()
                            .unwrap(),
                    )
                    .unwrap();
                    forward_mode_request(target_id, ModeRequest::SetMode(mode_submode));
                }
                Subservice::TcReadMode => {
                    let target_id = u32::from_be_bytes(app_data[0..4].try_into().unwrap());
                    forward_mode_request(target_id, ModeRequest::ReadMode);
                }
                Subservice::TcAnnounceMode => {
                    let target_id = u32::from_be_bytes(app_data[0..4].try_into().unwrap());
                    forward_mode_request(target_id, ModeRequest::AnnounceMode);
                }
                Subservice::TcAnnounceModeRecursive => {
                    let target_id = u32::from_be_bytes(app_data[0..4].try_into().unwrap());
                    forward_mode_request(target_id, ModeRequest::AnnounceModeRecursive);
                }
                _ => {
                    warn!("Can not process mode request with subservice {subservice:?}");
                    invalid_subservice_handler();
                    valid_subservice = false;
                }
            }
            if valid_subservice {
                self.tm_args
                    .verif_reporter
                    .start_success(token, Some(self.stamp_helper.stamp()))
                    .expect("sending start success TM failed");
            }
        } else {
            invalid_subservice_handler();
        }
    }
}
