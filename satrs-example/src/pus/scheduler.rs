use crate::tmtc::PusTcSource;
use log::{error, info, warn};
use satrs_core::pool::{PoolProviderMemInPlace, StaticMemoryPool};
use satrs_core::pus::scheduler::{PusScheduler, TcInfo};
use satrs_core::pus::scheduler_srv::PusService11SchedHandler;
use satrs_core::pus::{EcssTcInSharedStoreConverter, PusPacketHandlerResult};

pub struct Pus11Wrapper {
    pub pus_11_handler: PusService11SchedHandler<EcssTcInSharedStoreConverter, PusScheduler>,
    pub sched_tc_pool: StaticMemoryPool,
    pub tc_source_wrapper: PusTcSource,
}

impl Pus11Wrapper {
    pub fn release_tcs(&mut self) {
        let releaser = |enabled: bool, _info: &TcInfo, tc: &[u8]| -> bool {
            if enabled {
                // Transfer TC from scheduler TC pool to shared TC pool.
                let released_tc_addr = self
                    .tc_source_wrapper
                    .tc_store
                    .pool
                    .write()
                    .expect("locking pool failed")
                    .add(tc)
                    .expect("adding TC to shared pool failed");

                self.tc_source_wrapper
                    .tc_source
                    .send(released_tc_addr)
                    .expect("sending TC to TC source failed");
            }
            true
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
