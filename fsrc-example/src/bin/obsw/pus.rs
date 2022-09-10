use crate::tmtc::TmStore;
use fsrc_core::pool::StoreAddr;
use fsrc_core::pus::verification::SharedStdVerifReporterWithSender;
use fsrc_core::tmtc::tm_helper::PusTmWithCdsShortHelper;
use fsrc_core::tmtc::PusServiceProvider;
use spacepackets::tc::{PusTc, PusTcSecondaryHeaderT};
use spacepackets::time::{CdsShortTimeProvider, TimeWriter};
use spacepackets::SpHeader;
use std::sync::mpsc;

pub struct PusReceiver {
    pub tm_helper: PusTmWithCdsShortHelper,
    pub tm_tx: mpsc::Sender<StoreAddr>,
    pub tm_store: TmStore,
    pub verif_reporter: SharedStdVerifReporterWithSender,
    stamper: CdsShortTimeProvider,
    time_stamp: [u8; 7],
}

impl PusReceiver {
    pub fn new(
        apid: u16,
        tm_tx: mpsc::Sender<StoreAddr>,
        tm_store: TmStore,
        verif_reporter: SharedStdVerifReporterWithSender,
    ) -> Self {
        Self {
            tm_helper: PusTmWithCdsShortHelper::new(apid),
            tm_tx,
            tm_store,
            verif_reporter,
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
        let _accepted_token = reporter
            .acceptance_success(init_token, &self.time_stamp)
            .expect("Acceptance success failure");
        drop(reporter);
        if service == 17 {
            self.handle_test_service(pus_tc);
        }
        Ok(())
    }
}

impl PusReceiver {
    fn handle_test_service(&mut self, pus_tc: &PusTc) {
        if pus_tc.subservice() == 1 {
            println!("Received PUS ping command TC[17,1]");
            println!("Sending ping reply PUS TM[17,2]");
            let ping_reply = self.tm_helper.create_pus_tm_timestamp_now(17, 2, None);
            let addr = self.tm_store.add_pus_tm(&ping_reply);
            self.tm_tx
                .send(addr)
                .expect("Sending TM to TM funnel failed");
        }
    }
}
