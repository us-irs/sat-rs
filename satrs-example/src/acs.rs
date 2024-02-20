use std::sync::mpsc::{self, TryRecvError};

use log::{info, warn};
use satrs::pus::verification::{VerificationReporterWithSender, VerificationReportingProvider};
use satrs::pus::{EcssTmSender, PusTmWrapper};
use satrs::request::TargetAndApidId;
use satrs::spacepackets::ecss::hk::Subservice as HkSubservice;
use satrs::{
    hk::HkRequest,
    spacepackets::{
        ecss::tm::{PusTmCreator, PusTmSecondaryHeader},
        time::cds::{DaysLen16Bits, TimeProvider},
        SequenceFlags, SpHeader,
    },
};
use satrs_example::config::{RequestTargetId, PUS_APID};

use crate::{
    hk::{AcsHkIds, HkUniqueId},
    requests::{Request, RequestWithToken},
    update_time,
};

pub struct AcsTask {
    timestamp: [u8; 7],
    time_provider: TimeProvider<DaysLen16Bits>,
    verif_reporter: VerificationReporterWithSender,
    tm_sender: Box<dyn EcssTmSender>,
    request_rx: mpsc::Receiver<RequestWithToken>,
}

impl AcsTask {
    pub fn new(
        tm_sender: impl EcssTmSender,
        request_rx: mpsc::Receiver<RequestWithToken>,
        verif_reporter: VerificationReporterWithSender,
    ) -> Self {
        Self {
            timestamp: [0; 7],
            time_provider: TimeProvider::new_with_u16_days(0, 0),
            verif_reporter,
            tm_sender: Box::new(tm_sender),
            request_rx,
        }
    }

    fn handle_hk_request(&mut self, target_id: u32, unique_id: u32) {
        assert_eq!(target_id, RequestTargetId::AcsSubsystem as u32);
        if unique_id == AcsHkIds::TestMgmSet as u32 {
            let mut sp_header = SpHeader::tm(PUS_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
            let sec_header = PusTmSecondaryHeader::new_simple(
                3,
                HkSubservice::TmHkPacket as u8,
                &self.timestamp,
            );
            let mut buf: [u8; 8] = [0; 8];
            let hk_id = HkUniqueId::new(target_id, unique_id);
            hk_id.write_to_be_bytes(&mut buf).unwrap();
            let pus_tm = PusTmCreator::new(&mut sp_header, sec_header, &buf, true);
            self.tm_sender
                .send_tm(PusTmWrapper::Direct(pus_tm))
                .expect("Sending HK TM failed");
        }
        // TODO: Verification failure for invalid unique IDs.
    }

    pub fn try_reading_one_request(&mut self) -> bool {
        match self.request_rx.try_recv() {
            Ok(request) => {
                info!(
                    "ACS thread: Received HK request {:?}",
                    request.targeted_request
                );
                let target_and_apid_id = TargetAndApidId::from(request.targeted_request.target_id);
                match request.targeted_request.request {
                    Request::Hk(hk_req) => match hk_req {
                        HkRequest::OneShot(unique_id) => {
                            self.handle_hk_request(target_and_apid_id.target(), unique_id)
                        }
                        HkRequest::Enable(_) => {}
                        HkRequest::Disable(_) => {}
                        HkRequest::ModifyCollectionInterval(_, _) => {}
                    },
                    Request::Mode(_mode_req) => {
                        warn!("mode request handling not implemented yet")
                    }
                    Request::Action(_action_req) => {
                        warn!("action request handling not implemented yet")
                    }
                }
                let started_token = self
                    .verif_reporter
                    .start_success(request.token, &self.timestamp)
                    .expect("Sending start success failed");
                self.verif_reporter
                    .completion_success(started_token, &self.timestamp)
                    .expect("Sending completion success failed");
                true
            }
            Err(e) => match e {
                TryRecvError::Empty => false,
                TryRecvError::Disconnected => {
                    warn!("ACS thread: Message Queue TX disconnected!");
                    false
                }
            },
        }
    }

    pub fn periodic_operation(&mut self) {
        update_time(&mut self.time_provider, &mut self.timestamp);
        loop {
            if !self.try_reading_one_request() {
                break;
            }
        }
    }
}
