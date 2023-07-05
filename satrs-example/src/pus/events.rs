use satrs_core::event_man::{EventManager, EventManagerWithMpscQueue};
use satrs_core::events::EventU32;
use satrs_core::params::Params;
use satrs_core::pool::{SharedPool, StoreAddr};
use satrs_core::pus::event_man::EventReporter;
use satrs_core::pus::verification::{
    StdVerifReporterWithSender, TcStateAccepted, VerificationToken,
};
use satrs_core::pus::{
    AcceptedTc, PusPacketHandlerResult, PusPacketHandlingError, PusServiceBase, PusServiceHandler,
};
use satrs_core::tmtc::tm_helper::SharedTmStore;
use std::sync::mpsc::{Receiver, Sender};

pub struct PusService5EventHandler {
    psb: PusServiceBase,
    event_manager: EventManagerWithMpscQueue<EventU32, Params>,
}

impl PusService5EventHandler {
    pub fn new(
        receiver: Receiver<AcceptedTc>,
        tc_pool: SharedPool,
        tm_tx: Sender<StoreAddr>,
        tm_store: SharedTmStore,
        tm_apid: u16,
        verification_handler: StdVerifReporterWithSender,
        event_manager: EventManagerWithMpscQueue<EventU32, Params>,
    ) -> Self {
        Self {
            psb: PusServiceBase::new(
                receiver,
                tc_pool,
                tm_tx,
                tm_store,
                tm_apid,
                verification_handler,
            ),
            event_manager,
        }
    }
}

impl PusServiceHandler for PusService5EventHandler {
    fn psb_mut(&mut self) -> &mut PusServiceBase {
        &mut self.psb
    }
    fn psb(&self) -> &PusServiceBase {
        &self.psb
    }

    fn handle_one_tc(
        &mut self,
        addr: StoreAddr,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}
