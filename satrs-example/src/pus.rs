use crate::tmtc::TmStore;
use satrs_core::events::EventU32;
use satrs_core::pool::StoreAddr;
use satrs_core::pus::event::Subservices;
use satrs_core::pus::event_man::{EventRequest, EventRequestWithToken};
use satrs_core::pus::verification::{
    FailParams, SharedStdVerifReporterWithSender, TcStateAccepted, VerificationToken,
};
use satrs_core::tmtc::tm_helper::PusTmWithCdsShortHelper;
use satrs_core::tmtc::PusServiceProvider;
use spacepackets::ecss::{EcssEnumU16, PusPacket};
use spacepackets::tc::PusTc;
use spacepackets::time::{CdsShortTimeProvider, TimeWriter};
use spacepackets::SpHeader;
use std::sync::mpsc;

pub struct PusReceiver {
    pub tm_helper: PusTmWithCdsShortHelper,
    pub tm_tx: mpsc::Sender<StoreAddr>,
    pub tm_store: TmStore,
    pub verif_reporter: SharedStdVerifReporterWithSender,
    event_request_tx: mpsc::Sender<EventRequestWithToken>,
    stamper: CdsShortTimeProvider,
    time_stamp: [u8; 7],
}

impl PusReceiver {
    pub fn new(
        apid: u16,
        tm_tx: mpsc::Sender<StoreAddr>,
        tm_store: TmStore,
        verif_reporter: SharedStdVerifReporterWithSender,
        event_request_tx: mpsc::Sender<EventRequestWithToken>,
    ) -> Self {
        Self {
            tm_helper: PusTmWithCdsShortHelper::new(apid),
            tm_tx,
            tm_store,
            verif_reporter,
            event_request_tx,
            stamper: CdsShortTimeProvider::default(),
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
        let mut reporter = self
            .verif_reporter
            .lock()
            .expect("Locking Verification Reporter failed");
        let init_token = reporter.add_tc(pus_tc);
        self.stamper
            .update_from_now()
            .expect("Updating time for time stamp failed");
        self.stamper
            .write_to_bytes(&mut self.time_stamp)
            .expect("Writing time stamp failed");
        let accepted_token = reporter
            .acceptance_success(init_token, &self.time_stamp)
            .expect("Acceptance success failure");
        drop(reporter);
        if service == 17 {
            self.handle_test_service(pus_tc, accepted_token);
        } else if service == 5 {
            self.handle_event_service(pus_tc, accepted_token);
        }
        Ok(())
    }
}

impl PusReceiver {
    fn handle_test_service(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        if pus_tc.subservice() == 1 {
            println!("Received PUS ping command TC[17,1]");
            println!("Sending ping reply PUS TM[17,2]");
            let ping_reply = self.tm_helper.create_pus_tm_timestamp_now(17, 2, None);
            let addr = self.tm_store.add_pus_tm(&ping_reply);
            let mut reporter = self
                .verif_reporter
                .lock()
                .expect("Error locking verification reporter");
            let start_token = reporter
                .start_success(token, &self.time_stamp)
                .expect("Error sending start success");
            self.tm_tx
                .send(addr)
                .expect("Sending TM to TM funnel failed");
            reporter
                .completion_success(start_token, &self.time_stamp)
                .expect("Error sending completion success");
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

    fn handle_event_service(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
        let send_start_failure = |verif_reporter: &mut SharedStdVerifReporterWithSender,
                                  timestamp: &[u8; 7],
                                  failure_code: EcssEnumU16,
                                  failure_data: Option<&[u8]>| {
            verif_reporter
                .lock()
                .expect("Locking verification reporter failed")
                .start_failure(
                    token,
                    FailParams::new(timestamp, &failure_code, failure_data),
                )
                .expect("Sending start failure TM failed");
        };
        let send_start_acceptance = |verif_reporter: &mut SharedStdVerifReporterWithSender,
                                     timestamp: &[u8; 7]| {
            verif_reporter
                .lock()
                .expect("Locking verification reporter failed")
                .start_success(token, timestamp)
                .expect("Sending start success TM failed")
        };
        if pus_tc.user_data().is_none() {
            self.update_time_stamp();
            send_start_failure(
                &mut self.verif_reporter,
                &self.time_stamp,
                EcssEnumU16::new(1),
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
                EcssEnumU16::new(1),
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
                    EcssEnumU16::new(2),
                    None,
                );
            }
        }
    }
}
