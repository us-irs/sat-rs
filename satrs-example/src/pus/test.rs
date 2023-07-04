use crate::pus::{AcceptedTc, PusServiceBase};
use delegate::delegate;
use log::info;
use satrs_core::events::EventU32;
use satrs_core::params::Params;
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
use satrs_example::TEST_EVENT;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

pub struct SatrsTestServiceCustomHandler {
    pub event_sender: Sender<(EventU32, Option<Params>)>,
}

pub struct PusService17TestHandler {
    psb: PusServiceBase,
}

pub enum PacketHandlerResult {
    PingRequestHandled,
    CustomSubservice(VerificationToken<TcStateAccepted>),
    Empty,
}

impl PusService17TestHandler {
    pub fn new(
        receiver: Receiver<AcceptedTc>,
        tc_pool: SharedPool,
        tm_helper: PusTmWithCdsShortHelper,
        tm_tx: Sender<StoreAddr>,
        tm_store: SharedTmStore,
        verification_handler: StdVerifReporterWithSender,
    ) -> Self {
        Self {
            psb: PusServiceBase::new(
                receiver,
                tc_pool,
                tm_helper,
                tm_tx,
                tm_store,
                verification_handler,
            ),
        }
    }

    pub fn verification_handler(&mut self) -> &mut StdVerifReporterWithSender {
        &mut self.psb.verification_handler
    }

    pub fn pus_tc_buf(&self) -> (&[u8], usize) {
        (&self.psb.pus_buf, self.psb.pus_size)
    }

    // TODO: Return errors which occured
    pub fn periodic_operation(&mut self) -> Result<u32, ()> {
        self.psb.handled_tcs = 0;
        loop {
            match self.psb.tc_rx.try_recv() {
                Ok((addr, token)) => {
                    self.handle_one_tc(addr, token);
                }
                Err(e) => {
                    match e {
                        TryRecvError::Empty => return Ok(self.psb.handled_tcs),
                        TryRecvError::Disconnected => {
                            // TODO: Replace panic by something cleaner
                            panic!("PusService17Handler: Sender disconnected");
                        }
                    }
                }
            }
        }
    }

    pub fn handle_next_packet(&mut self) -> Result<PacketHandlerResult, ()> {
        match self.psb.tc_rx.try_recv() {
            Ok((addr, token)) => {
                if self.handle_one_tc(addr, token) {
                    return Ok(PacketHandlerResult::PingRequestHandled);
                } else {
                    return Ok(PacketHandlerResult::CustomSubservice);
                }
            }
            Err(e) => {
                match e {
                    TryRecvError::Empty => return Ok(PacketHandlerResult::Empty),
                    TryRecvError::Disconnected => {
                        // TODO: Replace panic by something cleaner
                        panic!("PusService17Handler: Sender disconnected");
                    }
                }
            }
        }
    }

    pub fn handle_one_tc(
        &mut self,
        addr: StoreAddr,
        token: VerificationToken<TcStateAccepted>,
    ) -> bool {
        let time_provider = TimeProvider::from_now_with_u16_days().unwrap();
        // TODO: Better error handling
        {
            // Keep locked section as short as possible.
            let mut tc_pool = self.psb.tc_store.write().unwrap();
            let tc_guard = tc_pool.read_with_guard(addr);
            let tc_raw = tc_guard.read().unwrap();
            self.psb.pus_buf[0..tc_raw.len()].copy_from_slice(tc_raw);
        }
        let (tc, tc_size) = PusTc::from_bytes(&self.psb.pus_buf).unwrap();
        // TODO: Robustness: Check that service is 17
        if tc.subservice() == 1 {
            info!("Received PUS ping command TC[17,1]");
            info!("Sending ping reply PUS TM[17,2]");
            time_provider
                .write_to_bytes(&mut self.psb.stamp_buf)
                .unwrap();
            let start_token = self
                .psb
                .verification_handler
                .start_success(token, Some(&self.psb.stamp_buf))
                .expect("Error sending start success");
            // Sequence count will be handled centrally in TM funnel.
            let ping_reply =
                self.psb
                    .tm_helper
                    .create_pus_tm_with_stamp(17, 2, None, &time_provider, 0);
            let addr = self.psb.tm_store.add_pus_tm(&ping_reply);
            self.psb
                .tm_tx
                .send(addr)
                .expect("Sending TM to TM funnel failed");
            self.psb
                .verification_handler
                .completion_success(start_token, Some(&self.psb.stamp_buf))
                .expect("Error sending completion success");
            self.psb.handled_tcs += 1;
            true
        }
        false
        // TODO: How to handle invalid subservice?
        // TODO: How do we handle custom code like this? Custom subservice handler via trait?
        // if tc.subservice() == 128 {
        //             info!("Generating test event");
        //             self.event_sender
        //                 .send((TEST_EVENT.into(), None))
        //                 .expect("Sending test event failed");
        //             let start_token =
        //                 verification_handler
        //                 .start_success(token, Some(&stamp_buf))
        //                 .expect("Error sending start success");
        //             verification_handler
        //                 .completion_success(start_token, Some(&stamp_buf))
        //                 .expect("Error sending completion success");
        //
    }
}
