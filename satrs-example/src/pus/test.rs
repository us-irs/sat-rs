use crate::pus::AcceptedTc;
use log::info;
use satrs_core::pool::{SharedPool, StoreAddr};
use satrs_core::pus::verification::{
    StdVerifReporterWithSender, TcStateAccepted, VerificationToken,
};
use satrs_core::seq_count::{SeqCountProviderSyncClonable, SequenceCountProviderCore};
use satrs_core::spacepackets::ecss::PusPacket;
use satrs_core::spacepackets::tc::PusTc;
use satrs_core::spacepackets::time::cds::TimeProvider;
use satrs_core::spacepackets::time::TimeWriter;
use satrs_core::spacepackets::tm::PusTm;
use satrs_core::tmtc::tm_helper::{PusTmWithCdsShortHelper, SharedTmStore};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

pub struct PusService17Handler {
    tc_rx: Receiver<AcceptedTc>,
    tc_store: SharedPool,
    tm_helper: PusTmWithCdsShortHelper,
    tm_tx: Sender<StoreAddr>,
    tm_store: SharedTmStore,
    verification_handler: StdVerifReporterWithSender,
    stamp_buf: [u8; 7],
    pus_buf: [u8; 2048],
    handled_tcs: u32,
}

impl PusService17Handler {
    pub fn new(receiver: Receiver<AcceptedTc>, tc_pool: SharedPool, tm_helper: PusTmWithCdsShortHelper, tm_tx: Sender<StoreAddr>, tm_store: SharedTmStore, verification_handler: StdVerifReporterWithSender) -> Self {
        Self {
            tc_rx: receiver,
            tc_store: tc_pool,
            tm_helper,
            tm_tx,
            tm_store,
            verification_handler,
            stamp_buf: [0; 7],
            pus_buf: [0; 2048],
            handled_tcs: 0
        }
    }
    // TODO: Return errors which occured
    pub fn periodic_operation(&mut self) -> Result<u32, ()> {
        self.handled_tcs = 0;
        loop {
            match self.tc_rx.try_recv() {
                Ok((addr, token)) => {
                    self.handle_one_tc(addr, token);
                }
                Err(e) => {
                    match e {
                        TryRecvError::Empty => return Ok(self.handled_tcs),
                        TryRecvError::Disconnected => {
                            // TODO: Replace panic by something cleaner
                            panic!("PusService17Handler: Sender disconnected");
                        }
                    }
                }
            }
        }
    }
    pub fn handle_one_tc(&mut self, addr: StoreAddr, token: VerificationToken<TcStateAccepted>) {
        let time_provider = TimeProvider::from_now_with_u16_days().unwrap();
        // TODO: Better error handling
        let (addr, token) = self.tc_rx.try_recv().unwrap();
        {
            // Keep locked section as short as possible.
            let mut tc_pool = self.tc_store.write().unwrap();
            let tc_guard = tc_pool.read_with_guard(addr);
            let tc_raw = tc_guard.read().unwrap();
            self.pus_buf[0..tc_raw.len()].copy_from_slice(tc_raw);
        }
        let tc = PusTc::from_bytes(&self.pus_buf).unwrap();
        // TODO: Robustness: Check that service is 17
        if tc.0.subservice() == 1 {
            info!("Received PUS ping command TC[17,1]");
            info!("Sending ping reply PUS TM[17,2]");
            time_provider.write_to_bytes(&mut self.stamp_buf).unwrap();
            let start_token = self
                .verification_handler
                .start_success(token, Some(&self.stamp_buf))
                .expect("Error sending start success");
            // Sequence count will be handled centrally in TM funnel.
            let ping_reply = self.tm_helper.create_pus_tm_with_stamp(17, 2, None, &time_provider, 0);
            let addr = self.tm_store.add_pus_tm(&ping_reply);
            self.tm_tx
                .send(addr)
                .expect("Sending TM to TM funnel failed");
            self.verification_handler
                .completion_success(start_token, Some(&self.stamp_buf))
                .expect("Error sending completion success");
            self.handled_tcs += 1;
        }
    }
}
