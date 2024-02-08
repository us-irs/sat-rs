use std::sync::mpsc;
use std::time::Duration;

use log::{error, info, warn};
use satrs_core::pool::{PoolProviderMemInPlace, StaticMemoryPool, StoreAddr};
use satrs_core::pus::scheduler::{PusScheduler, TcInfo};
use satrs_core::pus::scheduler_srv::PusService11SchedHandler;
use satrs_core::pus::verification::VerificationReporterWithSender;
use satrs_core::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter,
    MpscTcReceiver, MpscTmAsVecSender, MpscTmInSharedPoolSender, PusPacketHandlerResult,
    PusServiceHelper,
};
use satrs_core::tmtc::tm_helper::SharedTmPool;
use satrs_core::ChannelId;
use satrs_example::config::{TcReceiverId, TmSenderId, PUS_APID};

use crate::tmtc::PusTcSourceProviderSharedPool;

pub trait TcReleaser {
    fn release(&mut self, enabled: bool, info: &TcInfo, tc: &[u8]) -> bool;
}

impl TcReleaser for PusTcSourceProviderSharedPool {
    fn release(&mut self, enabled: bool, _info: &TcInfo, tc: &[u8]) -> bool {
        if enabled {
            // Transfer TC from scheduler TC pool to shared TC pool.
            let released_tc_addr = self
                .shared_pool
                .pool
                .write()
                .expect("locking pool failed")
                .add(tc)
                .expect("adding TC to shared pool failed");
            self.tc_source
                .send(released_tc_addr)
                .expect("sending TC to TC source failed");
        }
        true
    }
}

impl TcReleaser for mpsc::Sender<Vec<u8>> {
    fn release(&mut self, enabled: bool, _info: &TcInfo, tc: &[u8]) -> bool {
        if enabled {
            // Send released TC to centralized TC source.
            self.send(tc.to_vec())
                .expect("sending TC to TC source failed");
        }
        true
    }
}

pub struct Pus11Wrapper<TcInMemConverter: EcssTcInMemConverter> {
    pub pus_11_handler: PusService11SchedHandler<TcInMemConverter, PusScheduler>,
    pub sched_tc_pool: StaticMemoryPool,
    pub tc_releaser: Box<dyn TcReleaser + Send>,
}

impl<TcInMemConverter: EcssTcInMemConverter> Pus11Wrapper<TcInMemConverter> {
    pub fn release_tcs(&mut self) {
        let releaser = |enabled: bool, info: &TcInfo, tc: &[u8]| -> bool {
            self.tc_releaser.release(enabled, info, tc)
        };

        self.pus_11_handler
            .scheduler_mut()
            .update_time_from_now()
            .unwrap();
        let released_tcs = self
            .pus_11_handler
            .scheduler_mut()
            .release_telecommands(releaser, &mut self.sched_tc_pool)
            .expect("releasing TCs failed");
        if released_tcs > 0 {
            info!("{released_tcs} TC(s) released from scheduler");
        }
    }

    pub fn handle_next_packet(&mut self) -> bool {
        match self.pus_11_handler.handle_one_tc(&mut self.sched_tc_pool) {
            Ok(result) => match result {
                PusPacketHandlerResult::RequestHandled => {}
                PusPacketHandlerResult::RequestHandledPartialSuccess(e) => {
                    warn!("PUS11 partial packet handling success: {e:?}")
                }
                PusPacketHandlerResult::CustomSubservice(invalid, _) => {
                    warn!("PUS11 invalid subservice {invalid}");
                }
                PusPacketHandlerResult::SubserviceNotImplemented(subservice, _) => {
                    warn!("PUS11: Subservice {subservice} not implemented");
                }
                PusPacketHandlerResult::Empty => {
                    return true;
                }
            },
            Err(error) => {
                error!("PUS packet handling error: {error:?}")
            }
        }
        false
    }
}

pub fn create_scheduler_service_static(
    shared_tm_store: SharedTmPool,
    tm_funnel_tx: mpsc::Sender<StoreAddr>,
    verif_reporter: VerificationReporterWithSender,
    tc_releaser: PusTcSourceProviderSharedPool,
    pus_sched_rx: mpsc::Receiver<EcssTcAndToken>,
    sched_tc_pool: StaticMemoryPool,
) -> Pus11Wrapper<EcssTcInSharedStoreConverter> {
    let sched_srv_tm_sender = MpscTmInSharedPoolSender::new(
        TmSenderId::PusSched as ChannelId,
        "PUS_11_TM_SENDER",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    );
    let sched_srv_receiver = MpscTcReceiver::new(
        TcReceiverId::PusSched as ChannelId,
        "PUS_11_TC_RECV",
        pus_sched_rx,
    );
    let scheduler = PusScheduler::new_with_current_init_time(Duration::from_secs(5))
        .expect("Creating PUS Scheduler failed");
    let pus_11_handler = PusService11SchedHandler::new(
        PusServiceHelper::new(
            Box::new(sched_srv_receiver),
            Box::new(sched_srv_tm_sender),
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInSharedStoreConverter::new(tc_releaser.clone_backing_pool(), 2048),
        ),
        scheduler,
    );
    Pus11Wrapper {
        pus_11_handler,
        sched_tc_pool,
        tc_releaser: Box::new(tc_releaser),
    }
}

pub fn create_scheduler_service_dynamic(
    tm_funnel_tx: mpsc::Sender<Vec<u8>>,
    verif_reporter: VerificationReporterWithSender,
    tc_source_sender: mpsc::Sender<Vec<u8>>,
    pus_sched_rx: mpsc::Receiver<EcssTcAndToken>,
    sched_tc_pool: StaticMemoryPool,
) -> Pus11Wrapper<EcssTcInVecConverter> {
    let sched_srv_tm_sender = MpscTmAsVecSender::new(
        TmSenderId::PusSched as ChannelId,
        "PUS_11_TM_SENDER",
        tm_funnel_tx,
    );
    let sched_srv_receiver = MpscTcReceiver::new(
        TcReceiverId::PusSched as ChannelId,
        "PUS_11_TC_RECV",
        pus_sched_rx,
    );
    let scheduler = PusScheduler::new_with_current_init_time(Duration::from_secs(5))
        .expect("Creating PUS Scheduler failed");
    let pus_11_handler = PusService11SchedHandler::new(
        PusServiceHelper::new(
            Box::new(sched_srv_receiver),
            Box::new(sched_srv_tm_sender),
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInVecConverter::default(),
        ),
        scheduler,
    );
    Pus11Wrapper {
        pus_11_handler,
        sched_tc_pool,
        tc_releaser: Box::new(tc_source_sender),
    }
}
