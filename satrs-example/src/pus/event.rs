use std::sync::mpsc;

use crate::pus::create_verification_reporter;
use satrs::pool::SharedStaticMemoryPool;
use satrs::pus::event_man::EventRequestWithToken;
use satrs::pus::event_srv::PusEventServiceHandler;
use satrs::pus::verification::VerificationReporter;
use satrs::pus::{
    DirectPusPacketHandlerResult, EcssTcAndToken, EcssTcInMemConverter,
    EcssTcInSharedStoreConverter, EcssTcInVecConverter, EcssTmSender, MpscTcReceiver,
    MpscTmAsVecSender, PartialPusHandlingError, PusServiceHelper,
};
use satrs::spacepackets::ecss::PusServiceId;
use satrs::tmtc::{PacketAsVec, PacketSenderWithSharedPool};
use satrs_example::config::components::PUS_EVENT_MANAGEMENT;

use super::{DirectPusService, HandlingStatus};

pub fn create_event_service_static(
    tm_sender: PacketSenderWithSharedPool,
    tc_pool: SharedStaticMemoryPool,
    pus_event_rx: mpsc::Receiver<EcssTcAndToken>,
    event_request_tx: mpsc::Sender<EventRequestWithToken>,
) -> EventServiceWrapper<PacketSenderWithSharedPool, EcssTcInSharedStoreConverter> {
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
    tm_funnel_tx: mpsc::Sender<PacketAsVec>,
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

pub struct EventServiceWrapper<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter> {
    pub handler:
        PusEventServiceHandler<MpscTcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
}

impl<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter> DirectPusService
    for EventServiceWrapper<TmSender, TcInMemConverter>
{
    const SERVICE_ID: u8 = PusServiceId::Event as u8;

    const SERVICE_STR: &'static str = "events";

    fn poll_and_handle_next_tc(&mut self, time_stamp: &[u8]) -> HandlingStatus {
        let error_handler = |partial_error: &PartialPusHandlingError| {
            log::warn!(
                "PUS {}({}) partial error: {:?}",
                Self::SERVICE_ID,
                Self::SERVICE_STR,
                partial_error
            );
        };
        let result = self
            .handler
            .poll_and_handle_next_tc(error_handler, time_stamp);
        if let Err(e) = result {
            log::warn!(
                "PUS {}({}) error: {:?}",
                Self::SERVICE_ID,
                Self::SERVICE_STR,
                e
            );
            // To avoid permanent loops on continuous errors.
            return HandlingStatus::Empty;
        }
        match result.unwrap() {
            DirectPusPacketHandlerResult::Handled(handling_status) => return handling_status,
            DirectPusPacketHandlerResult::CustomSubservice(subservice, _) => {
                log::warn!(
                    "PUS {}({}) subservice {} not implemented",
                    Self::SERVICE_ID,
                    Self::SERVICE_STR,
                    subservice
                );
            }
            DirectPusPacketHandlerResult::SubserviceNotImplemented(subservice, _) => {
                log::warn!(
                    "PUS {}({}) subservice {} not implemented",
                    Self::SERVICE_ID,
                    Self::SERVICE_STR,
                    subservice
                );
            }
        }
        HandlingStatus::HandledOne
    }
}
