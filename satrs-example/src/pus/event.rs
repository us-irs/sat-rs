use std::sync::mpsc;

use crate::pus::create_verification_reporter;
use log::{error, warn};
use satrs::pool::SharedStaticMemoryPool;
use satrs::pus::event_man::EventRequestWithToken;
use satrs::pus::event_srv::PusEventServiceHandler;
use satrs::pus::verification::VerificationReporter;
use satrs::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter,
    EcssTmSenderCore, MpscTcReceiver, MpscTmAsVecSender, MpscTmInSharedPoolSenderBounded,
    PusPacketHandlerResult, PusServiceHelper, PusTmAsVec, PusTmInPool, TmInSharedPoolSender,
};
use satrs_example::config::components::PUS_EVENT_MANAGEMENT;

use super::HandlingStatus;

pub fn create_event_service_static(
    tm_sender: TmInSharedPoolSender<mpsc::SyncSender<PusTmInPool>>,
    tc_pool: SharedStaticMemoryPool,
    pus_event_rx: mpsc::Receiver<EcssTcAndToken>,
    event_request_tx: mpsc::Sender<EventRequestWithToken>,
) -> EventServiceWrapper<MpscTmInSharedPoolSenderBounded, EcssTcInSharedStoreConverter> {
    let pus_5_handler = PusEventServiceHandler::new(
        PusServiceHelper::new(
            PUS_EVENT_MANAGEMENT.id(),
            pus_event_rx,
            tm_sender,
            create_verification_reporter(PUS_EVENT_MANAGEMENT.id(), PUS_EVENT_MANAGEMENT.apid),
            EcssTcInSharedStoreConverter::new(tc_pool.clone(), 2048),
        ),
        event_request_tx,
    );
    EventServiceWrapper {
        handler: pus_5_handler,
    }
}

pub fn create_event_service_dynamic(
    tm_funnel_tx: mpsc::Sender<PusTmAsVec>,
    pus_event_rx: mpsc::Receiver<EcssTcAndToken>,
    event_request_tx: mpsc::Sender<EventRequestWithToken>,
) -> EventServiceWrapper<MpscTmAsVecSender, EcssTcInVecConverter> {
    let pus_5_handler = PusEventServiceHandler::new(
        PusServiceHelper::new(
            PUS_EVENT_MANAGEMENT.id(),
            pus_event_rx,
            tm_funnel_tx,
            create_verification_reporter(PUS_EVENT_MANAGEMENT.id(), PUS_EVENT_MANAGEMENT.apid),
            EcssTcInVecConverter::default(),
        ),
        event_request_tx,
    );
    EventServiceWrapper {
        handler: pus_5_handler,
    }
}

pub struct EventServiceWrapper<TmSender: EcssTmSenderCore, TcInMemConverter: EcssTcInMemConverter> {
    pub handler:
        PusEventServiceHandler<MpscTcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
}

impl<TmSender: EcssTmSenderCore, TcInMemConverter: EcssTcInMemConverter>
    EventServiceWrapper<TmSender, TcInMemConverter>
{
    pub fn poll_and_handle_next_tc(&mut self, time_stamp: &[u8]) -> HandlingStatus {
        match self.handler.poll_and_handle_next_tc(time_stamp) {
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
                PusPacketHandlerResult::Empty => return HandlingStatus::Empty,
            },
            Err(error) => {
                error!("PUS packet handling error: {error:?}")
            }
        }
        HandlingStatus::HandledOne
    }
}
