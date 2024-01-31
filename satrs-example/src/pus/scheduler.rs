use crate::tmtc::PusTcSource;
use log::{error, info, warn};
use satrs_core::pus::scheduler::TcInfo;
use satrs_core::pus::scheduler_srv::PusService11SchedHandler;
use satrs_core::pus::{EcssTcInStoreConverter, PusPacketHandlerResult};

pub struct Pus11Wrapper {
    pub pus_11_handler: PusService11SchedHandler<EcssTcInStoreConverter>,
    pub tc_source_wrapper: PusTcSource,
}

impl Pus11Wrapper {
    pub fn release_tcs(&mut self) {
        let releaser = |enabled: bool, info: &TcInfo| -> bool {
            if enabled {
                self.tc_source_wrapper
                    .tc_source
                    .send(info.addr())
                    .expect("sending TC to TC source failed");
            }
            true
        };

        let mut pool = self
            .tc_source_wrapper
            .tc_store
            .pool
            .write()
            .expect("error locking pool");

        self.pus_11_handler
            .scheduler_mut()
            .update_time_from_now()
            .unwrap();
        if let Ok(released_tcs) = self
            .pus_11_handler
            .scheduler_mut()
            .release_telecommands(releaser, pool.as_mut())
        {
            if released_tcs > 0 {
                info!("{released_tcs} TC(s) released from scheduler");
            }
        }
    }

    pub fn handle_next_packet(&mut self) -> bool {
        match self.pus_11_handler.handle_one_tc() {
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
