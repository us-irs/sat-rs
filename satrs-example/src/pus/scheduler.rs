use log::{error, warn};
use satrs_core::pus::scheduler_srv::PusService11SchedHandler;
use satrs_core::pus::{PusPacketHandlerResult, PusServiceHandler};

pub struct Pus11Wrapper {
    pub pus11_handler: PusService11SchedHandler,
}

impl Pus11Wrapper {
    pub fn perform_operation(&mut self) -> bool {
        match self.pus11_handler.handle_next_packet() {
            Ok(result) => match result {
                PusPacketHandlerResult::RequestHandled => {}
                PusPacketHandlerResult::RequestHandledPartialSuccess(e) => {
                    warn!("PUS11 partial packet handling success: {e:?}")
                }
                PusPacketHandlerResult::CustomSubservice(invalid, _) => {
                    warn!("PUS11 invalid subservice {invalid}");
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
