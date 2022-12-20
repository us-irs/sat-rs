use crate::hk::{CollectionIntervalFactor, HkRequest};
use crate::requests::Request;
use crate::tmtc::TmStore;
use satrs_core::events::EventU32;
use satrs_core::pool::StoreAddr;
use satrs_core::pus::event::Subservices;
use satrs_core::pus::event_man::{EventRequest, EventRequestWithToken};
use satrs_core::pus::hk;
use satrs_core::pus::verification::{
    FailParams, StdVerifReporterWithSender, TcStateAccepted, VerificationToken,
};
use satrs_core::res_code::ResultU16;
use satrs_core::tmtc::tm_helper::PusTmWithCdsShortHelper;
use satrs_core::tmtc::{AddressableId, PusServiceProvider};
use satrs_example::{hk_err, tmtc_err};
use spacepackets::ecss::PusPacket;
use spacepackets::tc::PusTc;
use spacepackets::time::cds::TimeProvider;
use spacepackets::time::TimeWriter;
use spacepackets::SpHeader;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

pub struct PusReceiver {
    pub tm_helper: PusTmWithCdsShortHelper,
    pub tm_tx: Sender<StoreAddr>,
    pub tm_store: TmStore,
    pub verif_reporter: StdVerifReporterWithSender,
    event_request_tx: Sender<EventRequestWithToken>,
    request_map: HashMap<u32, Sender<Request>>,
    stamper: TimeProvider,
    time_stamp: [u8; 7],
}

impl PusReceiver {
    pub fn new(
        apid: u16,
        tm_tx: Sender<StoreAddr>,
        tm_store: TmStore,
        verif_reporter: StdVerifReporterWithSender,
        event_request_tx: Sender<EventRequestWithToken>,
        request_map: HashMap<u32, Sender<Request>>,
    ) -> Self {
        Self {
            tm_helper: PusTmWithCdsShortHelper::new(apid),
            tm_tx,
            tm_store,
            verif_reporter,
            event_request_tx,
            request_map,
            stamper: TimeProvider::default(),
            time_stamp: [0; 7],
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
            .acceptance_success(init_token, &self.time_stamp)
            .expect("Acceptance success failure");
        if service == 17 {
            self.handle_test_service(pus_tc, accepted_token);
        } else if service == 5 {
            self.handle_event_request(pus_tc, accepted_token);
        } else if service == 3 {
            self.handle_hk_request(pus_tc, accepted_token);
        } else {
            self.update_time_stamp();
            self.verif_reporter
                .start_failure(
                    accepted_token,
                    FailParams::new(&self.time_stamp, &tmtc_err::INVALID_PUS_SERVICE, None),
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
                .start_success(token, &self.time_stamp)
                .expect("Error sending start success");
            self.tm_tx
                .send(addr)
                .expect("Sending TM to TM funnel failed");
            self.verif_reporter
                .completion_success(start_token, &self.time_stamp)
                .expect("Error sending completion success");
        } else {
            self.update_time_stamp();
            self.verif_reporter
                .start_failure(
                    token,
                    FailParams::new(&self.time_stamp, &tmtc_err::INVALID_PUS_SUBSERVICE, None),
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
                    FailParams::new(&self.time_stamp, &tmtc_err::NOT_ENOUGH_APP_DATA, None),
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
                .start_failure(token, FailParams::new(&self.time_stamp, err, None))
                .expect("Sending start failure TM failed");
            return;
        }
        let addressable_id = AddressableId::from_raw_be(user_data).unwrap();
        if !self.request_map.contains_key(&addressable_id.target_id) {
            self.update_time_stamp();
            self.verif_reporter
                .start_failure(
                    token,
                    FailParams::new(&self.time_stamp, &hk_err::UNKNOWN_TARGET_ID, None),
                )
                .expect("Sending start failure TM failed");
            return;
        }
        let send_request = |request: HkRequest| {
            let sender = self.request_map.get(&addressable_id.target_id).unwrap();
            sender
                .send(Request::HkRequest(request))
                .expect(&format!("Sending HK request {:?} failed", request))
        };
        if PusPacket::subservice(pus_tc) == hk::Subservice::TcEnableGeneration as u8 {
            send_request(HkRequest::Enable(addressable_id.unique_id));
        } else if PusPacket::subservice(pus_tc) == hk::Subservice::TcDisableGeneration as u8 {
            send_request(HkRequest::Disable(addressable_id.unique_id));
        } else if PusPacket::subservice(pus_tc) == hk::Subservice::TcGenerateOneShotHk as u8 {
            send_request(HkRequest::OneShot(addressable_id.unique_id));
        } else if PusPacket::subservice(pus_tc) == hk::Subservice::TcModifyCollectionInterval as u8
        {
            if user_data.len() < 12 {
                self.update_time_stamp();
                self.verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(
                            &self.time_stamp,
                            &hk_err::COLLECTION_INTERVAL_MISSING,
                            None,
                        ),
                    )
                    .expect("Sending start failure TM failed");
                return;
            }
            send_request(HkRequest::ModifyCollectionInterval(
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
                    FailParams::new(timestamp, failure_code, failure_data),
                )
                .expect("Sending start failure TM failed");
        };
        let send_start_acceptance = |verif_reporter: &mut StdVerifReporterWithSender,
                                     timestamp: &[u8; 7]| {
            verif_reporter
                .start_success(token, timestamp)
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
            Ok(Subservices::TcEnableEventGeneration) => {
                self.update_time_stamp();
                let start_token = send_start_acceptance(&mut self.verif_reporter, &self.time_stamp);
                self.event_request_tx
                    .send(EventRequestWithToken {
                        request: EventRequest::Enable(event_id),
                        token: start_token,
                    })
                    .expect("Sending event request failed");
            }
            Ok(Subservices::TcDisableEventGeneration) => {
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
}
