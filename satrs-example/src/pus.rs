use crate::hk::{CollectionIntervalFactor, HkRequest};
use crate::requests::{Request, RequestWithToken};
use crate::tmtc::{PusTcSource, TmStore};
use satrs_core::events::EventU32;
use satrs_core::pool::StoreAddr;
use satrs_core::pus::event;
use satrs_core::pus::event_man::{EventRequest, EventRequestWithToken};
use satrs_core::pus::hk;
use satrs_core::pus::scheduling::PusScheduler;
use satrs_core::pus::verification::{
    FailParams, StdVerifReporterWithSender, TcStateAccepted, VerificationToken,
};
use satrs_core::res_code::ResultU16;
use satrs_core::spacepackets::ecss::scheduling;
use satrs_core::tmtc::tm_helper::PusTmWithCdsShortHelper;
use satrs_core::tmtc::{AddressableId, PusServiceProvider};
use satrs_core::{
    spacepackets::ecss::PusPacket, spacepackets::tc::PusTc, spacepackets::time::cds::TimeProvider,
    spacepackets::time::TimeWriter, spacepackets::SpHeader,
};
use satrs_example::{hk_err, tmtc_err};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;

pub struct PusReceiver {
    pub tm_helper: PusTmWithCdsShortHelper,
    pub tm_tx: Sender<StoreAddr>,
    pub tm_store: TmStore,
    pub verif_reporter: StdVerifReporterWithSender,
    #[allow(dead_code)]
    tc_source: PusTcSource,
    event_request_tx: Sender<EventRequestWithToken>,
    request_map: HashMap<u32, Sender<RequestWithToken>>,
    stamper: TimeProvider,
    time_stamp: [u8; 7],
    scheduler: Rc<RefCell<PusScheduler>>,
}

pub struct PusTmArgs {
    /// All telemetry is sent with this sender handle.
    pub tm_tx: Sender<StoreAddr>,
    /// All TM to be sent is stored here
    pub tm_store: TmStore,
    /// All verification reporting is done with this reporter.
    pub verif_reporter: StdVerifReporterWithSender,
}

pub struct PusTcArgs {
    pub event_request_tx: Sender<EventRequestWithToken>,
    /// Request routing helper. Maps targeted request to their recipient.
    pub request_map: HashMap<u32, Sender<RequestWithToken>>,
    /// Required for scheduling of telecommands.
    pub tc_source: PusTcSource,
    pub scheduler: Rc<RefCell<PusScheduler>>,
}

impl PusReceiver {
    pub fn new(apid: u16, tm_arguments: PusTmArgs, tc_arguments: PusTcArgs) -> Self {
        Self {
            tm_helper: PusTmWithCdsShortHelper::new(apid),
            tm_tx: tm_arguments.tm_tx,
            tm_store: tm_arguments.tm_store,
            verif_reporter: tm_arguments.verif_reporter,
            tc_source: tc_arguments.tc_source,
            event_request_tx: tc_arguments.event_request_tx,
            request_map: tc_arguments.request_map,
            stamper: TimeProvider::new_with_u16_days(0, 0),
            time_stamp: [0; 7],
            scheduler: tc_arguments.scheduler,
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
        let init_token = self.verif_reporter.add_tc(pus_tc);
        self.update_time_stamp();
        let accepted_token = self
            .verif_reporter
            .acceptance_success(init_token, Some(&self.time_stamp))
            .expect("Acceptance success failure");
        if service == 17 {
            self.handle_test_service(pus_tc, accepted_token);
        } else if service == 5 {
            self.handle_event_request(pus_tc, accepted_token);
        } else if service == 3 {
            self.handle_hk_request(pus_tc, accepted_token);
        } else if service == 11 {
            self.handle_scheduled_tc(pus_tc, accepted_token);
        } else {
            self.update_time_stamp();
            self.verif_reporter
                .start_failure(
                    accepted_token,
                    FailParams::new(Some(&self.time_stamp), &tmtc_err::INVALID_PUS_SERVICE, None),
                )
                .expect("Start failure verification failed")
        }
        Ok(())
    }
}

impl PusReceiver {
    fn handle_test_service(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        if PusPacket::subservice(pus_tc) == 1 {
            println!("Received PUS ping command TC[17,1]");
            println!("Sending ping reply PUS TM[17,2]");
            let ping_reply = self.tm_helper.create_pus_tm_timestamp_now(17, 2, None);
            let addr = self.tm_store.add_pus_tm(&ping_reply);
            let start_token = self
                .verif_reporter
                .start_success(token, Some(&self.time_stamp))
                .expect("Error sending start success");
            self.tm_tx
                .send(addr)
                .expect("Sending TM to TM funnel failed");
            self.verif_reporter
                .completion_success(start_token, Some(&self.time_stamp))
                .expect("Error sending completion success");
        } else {
            self.update_time_stamp();
            self.verif_reporter
                .start_failure(
                    token,
                    FailParams::new(
                        Some(&self.time_stamp),
                        &tmtc_err::INVALID_PUS_SUBSERVICE,
                        None,
                    ),
                )
                .expect("Sending start failure TM failed");
        }
    }

    fn update_time_stamp(&mut self) {
        self.stamper
            .update_from_now()
            .expect("Updating timestamp failed");
        self.stamper
            .write_to_bytes(&mut self.time_stamp)
            .expect("Writing timestamp failed");
    }

    fn handle_hk_request(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        if pus_tc.user_data().is_none() {
            self.update_time_stamp();
            self.verif_reporter
                .start_failure(
                    token,
                    FailParams::new(Some(&self.time_stamp), &tmtc_err::NOT_ENOUGH_APP_DATA, None),
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
            self.update_time_stamp();
            self.verif_reporter
                .start_failure(token, FailParams::new(Some(&self.time_stamp), err, None))
                .expect("Sending start failure TM failed");
            return;
        }
        let addressable_id = AddressableId::from_raw_be(user_data).unwrap();
        if !self.request_map.contains_key(&addressable_id.target_id) {
            self.update_time_stamp();
            self.verif_reporter
                .start_failure(
                    token,
                    FailParams::new(Some(&self.time_stamp), &hk_err::UNKNOWN_TARGET_ID, None),
                )
                .expect("Sending start failure TM failed");
            return;
        }
        let send_request = |request: HkRequest| {
            let sender = self.request_map.get(&addressable_id.target_id).unwrap();
            sender
                .send(RequestWithToken(Request::HkRequest(request), token))
                .unwrap_or_else(|_| panic!("Sending HK request {request:?} failed"));
        };
        if PusPacket::subservice(pus_tc) == hk::Subservice::TcEnableHkGeneration as u8 {
            send_request(HkRequest::Enable(addressable_id));
        } else if PusPacket::subservice(pus_tc) == hk::Subservice::TcDisableHkGeneration as u8 {
            send_request(HkRequest::Disable(addressable_id));
        } else if PusPacket::subservice(pus_tc) == hk::Subservice::TcGenerateOneShotHk as u8 {
            send_request(HkRequest::OneShot(addressable_id));
        } else if PusPacket::subservice(pus_tc)
            == hk::Subservice::TcModifyHkCollectionInterval as u8
        {
            if user_data.len() < 12 {
                self.update_time_stamp();
                self.verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(
                            Some(&self.time_stamp),
                            &hk_err::COLLECTION_INTERVAL_MISSING,
                            None,
                        ),
                    )
                    .expect("Sending start failure TM failed");
                return;
            }
            send_request(HkRequest::ModifyCollectionInterval(
                addressable_id,
                CollectionIntervalFactor::from_be_bytes(user_data[8..12].try_into().unwrap()),
            ));
        }
    }

    fn handle_event_request(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        let send_start_failure = |verif_reporter: &mut StdVerifReporterWithSender,
                                  timestamp: &[u8; 7],
                                  failure_code: &ResultU16,
                                  failure_data: Option<&[u8]>| {
            verif_reporter
                .start_failure(
                    token,
                    FailParams::new(Some(timestamp), failure_code, failure_data),
                )
                .expect("Sending start failure TM failed");
        };
        let send_start_acceptance = |verif_reporter: &mut StdVerifReporterWithSender,
                                     timestamp: &[u8; 7]| {
            verif_reporter
                .start_success(token, Some(timestamp))
                .expect("Sending start success TM failed")
        };
        if pus_tc.user_data().is_none() {
            self.update_time_stamp();
            send_start_failure(
                &mut self.verif_reporter,
                &self.time_stamp,
                &tmtc_err::NOT_ENOUGH_APP_DATA,
                None,
            );
            return;
        }
        let app_data = pus_tc.user_data().unwrap();
        if app_data.len() < 4 {
            self.update_time_stamp();
            send_start_failure(
                &mut self.verif_reporter,
                &self.time_stamp,
                &tmtc_err::NOT_ENOUGH_APP_DATA,
                None,
            );
            return;
        }
        let event_id = EventU32::from(u32::from_be_bytes(app_data.try_into().unwrap()));
        match PusPacket::subservice(pus_tc).try_into() {
            Ok(event::Subservice::TcEnableEventGeneration) => {
                self.update_time_stamp();
                let start_token = send_start_acceptance(&mut self.verif_reporter, &self.time_stamp);
                self.event_request_tx
                    .send(EventRequestWithToken {
                        request: EventRequest::Enable(event_id),
                        token: start_token,
                    })
                    .expect("Sending event request failed");
            }
            Ok(event::Subservice::TcDisableEventGeneration) => {
                self.update_time_stamp();
                let start_token = send_start_acceptance(&mut self.verif_reporter, &self.time_stamp);
                self.event_request_tx
                    .send(EventRequestWithToken {
                        request: EventRequest::Disable(event_id),
                        token: start_token,
                    })
                    .expect("Sending event request failed");
            }
            _ => {
                self.update_time_stamp();
                send_start_failure(
                    &mut self.verif_reporter,
                    &self.time_stamp,
                    &tmtc_err::INVALID_PUS_SUBSERVICE,
                    None,
                );
            }
        }
    }

    fn handle_scheduled_tc(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        if pus_tc.user_data().is_none() {
            self.update_time_stamp();
            self.verif_reporter
                .start_failure(
                    token,
                    FailParams::new(Some(&self.time_stamp), &tmtc_err::NOT_ENOUGH_APP_DATA, None),
                )
                .expect("Sending start failure TM failed");
            return;
        }

        self.update_time_stamp();

        let subservice: scheduling::Subservice = match pus_tc.subservice().try_into() {
            Ok(subservice) => subservice,
            Err(_) => {
                self.verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(
                            Some(&self.time_stamp),
                            &tmtc_err::NOT_ENOUGH_APP_DATA,
                            None,
                        ),
                    )
                    .expect("Sending start failure TM failed");
                return;
            }
        };

        match subservice {
            scheduling::Subservice::TcEnableScheduling => {
                let start_token = self
                    .verif_reporter
                    .start_success(token, Some(&self.time_stamp))
                    .expect("Error sending start success");

                let mut scheduler = self.scheduler.borrow_mut();
                scheduler.enable();
                if scheduler.is_enabled() {
                    self.verif_reporter
                        .completion_success(start_token, Some(&self.time_stamp))
                        .expect("Error sending completion success");
                } else {
                    panic!("Failed to enable scheduler");
                }
                drop(scheduler);
            }
            scheduling::Subservice::TcDisableScheduling => {
                let start_token = self
                    .verif_reporter
                    .start_success(token, Some(&self.time_stamp))
                    .expect("Error sending start success");

                let mut scheduler = self.scheduler.borrow_mut();
                scheduler.disable();
                if !scheduler.is_enabled() {
                    self.verif_reporter
                        .completion_success(start_token, Some(&self.time_stamp))
                        .expect("Error sending completion success");
                } else {
                    panic!("Failed to disable scheduler");
                }
                drop(scheduler);
            }
            scheduling::Subservice::TcResetScheduling => {
                let start_token = self
                    .verif_reporter
                    .start_success(token, Some(&self.time_stamp))
                    .expect("Error sending start success");

                let mut pool = self
                    .tc_source
                    .tc_store
                    .pool
                    .write()
                    .expect("Locking pool failed");

                let mut scheduler = self.scheduler.borrow_mut();
                scheduler
                    .reset(pool.as_mut())
                    .expect("Error resetting TC Pool");
                drop(scheduler);

                self.verif_reporter
                    .completion_success(start_token, Some(&self.time_stamp))
                    .expect("Error sending completion success");
            }
            scheduling::Subservice::TcInsertActivity => {
                let start_token = self
                    .verif_reporter
                    .start_success(token, Some(&self.time_stamp))
                    .expect("Error sending start success");

                let mut pool = self
                    .tc_source
                    .tc_store
                    .pool
                    .write()
                    .expect("Locking pool failed");
                let mut scheduler = self.scheduler.borrow_mut();
                scheduler
                    .insert_wrapped_tc::<TimeProvider>(pus_tc, pool.as_mut())
                    .expect("TODO: panic message");
                drop(scheduler);

                self.verif_reporter
                    .completion_success(start_token, Some(&self.time_stamp))
                    .expect("Error sending completion success");
            }
            _ => {}
        }
    }
}
