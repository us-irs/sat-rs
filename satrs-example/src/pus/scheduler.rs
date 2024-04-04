use std::sync::mpsc;
use std::time::Duration;

use crate::pus::create_verification_reporter;
use log::{error, info, warn};
use satrs::pool::{PoolProvider, StaticMemoryPool};
use satrs::pus::scheduler::{PusScheduler, TcInfo};
use satrs::pus::scheduler_srv::PusSchedServiceHandler;
use satrs::pus::verification::VerificationReporter;
use satrs::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter,
    EcssTmSenderCore, MpscTcReceiver, MpscTmAsVecSender, MpscTmInSharedPoolSenderBounded,
    PusPacketHandlerResult, PusServiceHelper, PusTmAsVec, PusTmInPool, TmInSharedPoolSender,
};
use satrs_example::config::components::PUS_SCHED_SERVICE;

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

pub struct SchedulingServiceWrapper<
    TmSender: EcssTmSenderCore,
    TcInMemConverter: EcssTcInMemConverter,
> {
    pub pus_11_handler: PusSchedServiceHandler<
        MpscTcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        PusScheduler,
    >,
    pub sched_tc_pool: StaticMemoryPool,
    pub releaser_buf: [u8; 4096],
    pub tc_releaser: Box<dyn TcReleaser + Send>,
}

impl<TmSender: EcssTmSenderCore, TcInMemConverter: EcssTcInMemConverter>
    SchedulingServiceWrapper<TmSender, TcInMemConverter>
{
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
            .release_telecommands_with_buffer(
                releaser,
                &mut self.sched_tc_pool,
                &mut self.releaser_buf,
            )
            .expect("releasing TCs failed");
        if released_tcs > 0 {
            info!("{released_tcs} TC(s) released from scheduler");
        }
    }

    pub fn poll_and_handle_next_tc(&mut self, time_stamp: &[u8]) -> bool {
        match self
            .pus_11_handler
            .poll_and_handle_next_tc(time_stamp, &mut self.sched_tc_pool)
        {
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
    tm_sender: TmInSharedPoolSender<mpsc::SyncSender<PusTmInPool>>,
    tc_releaser: PusTcSourceProviderSharedPool,
    pus_sched_rx: mpsc::Receiver<EcssTcAndToken>,
    sched_tc_pool: StaticMemoryPool,
) -> SchedulingServiceWrapper<MpscTmInSharedPoolSenderBounded, EcssTcInSharedStoreConverter> {
    let scheduler = PusScheduler::new_with_current_init_time(Duration::from_secs(5))
        .expect("Creating PUS Scheduler failed");
    let pus_11_handler = PusSchedServiceHandler::new(
        PusServiceHelper::new(
            PUS_SCHED_SERVICE.id(),
            pus_sched_rx,
            tm_sender,
            create_verification_reporter(PUS_SCHED_SERVICE.id(), PUS_SCHED_SERVICE.apid),
            EcssTcInSharedStoreConverter::new(tc_releaser.clone_backing_pool(), 2048),
        ),
        scheduler,
    );
    SchedulingServiceWrapper {
        pus_11_handler,
        sched_tc_pool,
        releaser_buf: [0; 4096],
        tc_releaser: Box::new(tc_releaser),
    }
}

pub fn create_scheduler_service_dynamic(
    tm_funnel_tx: mpsc::Sender<PusTmAsVec>,
    tc_source_sender: mpsc::Sender<Vec<u8>>,
    pus_sched_rx: mpsc::Receiver<EcssTcAndToken>,
    sched_tc_pool: StaticMemoryPool,
) -> SchedulingServiceWrapper<MpscTmAsVecSender, EcssTcInVecConverter> {
    //let sched_srv_receiver =
    //MpscTcReceiver::new(PUS_SCHED_SERVICE.raw(), "PUS_11_TC_RECV", pus_sched_rx);
    let scheduler = PusScheduler::new_with_current_init_time(Duration::from_secs(5))
        .expect("Creating PUS Scheduler failed");
    let pus_11_handler = PusSchedServiceHandler::new(
        PusServiceHelper::new(
            PUS_SCHED_SERVICE.id(),
            pus_sched_rx,
            tm_funnel_tx,
            create_verification_reporter(PUS_SCHED_SERVICE.id(), PUS_SCHED_SERVICE.apid),
            EcssTcInVecConverter::default(),
        ),
        scheduler,
    );
    SchedulingServiceWrapper {
        pus_11_handler,
        sched_tc_pool,
        releaser_buf: [0; 4096],
        tc_releaser: Box::new(tc_source_sender),
    }
}
