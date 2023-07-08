use log::{error, warn};
use satrs_core::pus::event_srv::PusService5EventHandler;
use satrs_core::pus::{PusPacketHandlerResult, PusServiceHandler};

pub struct Pus5Wrapper {
    pub pus_5_handler: PusService5EventHandler,
}

impl Pus5Wrapper {
    pub fn handle_next_packet(&mut self) -> bool {
        match self.pus_5_handler.handle_next_packet() {
            Ok(result) => match result {
                PusPacketHandlerResult::RequestHandled => {}
                PusPacketHandlerResult::RequestHandledPartialSuccess(e) => {
                    warn!("PUS 5 partial packet handling success: {e:?}")
                }
                PusPacketHandlerResult::CustomSubservice(invalid, _) => {
                    warn!("PUS 5 invalid subservice {invalid}");
                }
                PusPacketHandlerResult::SubserviceNotImplemented(subservice, _) => {
                    warn!("PUS 5 subservice {subservice} not implemented");
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
