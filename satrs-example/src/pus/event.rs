use std::sync::mpsc;

use log::{error, warn};
use satrs_core::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs_core::pus::event_man::EventRequestWithToken;
use satrs_core::pus::event_srv::PusService5EventHandler;
use satrs_core::pus::verification::VerificationReporterWithSender;
use satrs_core::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter,
    MpscTcReceiver, MpscTmAsVecSender, MpscTmInSharedPoolSender, PusPacketHandlerResult,
    PusServiceHelper,
};
use satrs_core::tmtc::tm_helper::SharedTmPool;
use satrs_core::ChannelId;
use satrs_example::config::{TcReceiverId, TmSenderId, PUS_APID};

pub fn create_event_service_static(
    shared_tm_store: SharedTmPool,
    tm_funnel_tx: mpsc::Sender<StoreAddr>,
    verif_reporter: VerificationReporterWithSender,
    tc_pool: SharedStaticMemoryPool,
    pus_event_rx: mpsc::Receiver<EcssTcAndToken>,
    event_request_tx: mpsc::Sender<EventRequestWithToken>,
) -> Pus5Wrapper<EcssTcInSharedStoreConverter> {
    let event_srv_tm_sender = MpscTmInSharedPoolSender::new(
        TmSenderId::PusEvent as ChannelId,
        "PUS_5_TM_SENDER",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    );
    let event_srv_receiver = MpscTcReceiver::new(
        TcReceiverId::PusEvent as ChannelId,
        "PUS_5_TC_RECV",
        pus_event_rx,
    );
    let pus_5_handler = PusService5EventHandler::new(
        PusServiceHelper::new(
            Box::new(event_srv_receiver),
            Box::new(event_srv_tm_sender),
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInSharedStoreConverter::new(tc_pool.clone(), 2048),
        ),
        event_request_tx,
    );
    Pus5Wrapper { pus_5_handler }
}

pub fn create_event_service_dynamic(
    tm_funnel_tx: mpsc::Sender<Vec<u8>>,
    verif_reporter: VerificationReporterWithSender,
    pus_event_rx: mpsc::Receiver<EcssTcAndToken>,
    event_request_tx: mpsc::Sender<EventRequestWithToken>,
) -> Pus5Wrapper<EcssTcInVecConverter> {
    let event_srv_tm_sender = MpscTmAsVecSender::new(
        TmSenderId::PusEvent as ChannelId,
        "PUS_5_TM_SENDER",
        tm_funnel_tx,
    );
    let event_srv_receiver = MpscTcReceiver::new(
        TcReceiverId::PusEvent as ChannelId,
        "PUS_5_TC_RECV",
        pus_event_rx,
    );
    let pus_5_handler = PusService5EventHandler::new(
        PusServiceHelper::new(
            Box::new(event_srv_receiver),
            Box::new(event_srv_tm_sender),
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInVecConverter::default(),
        ),
        event_request_tx,
    );
    Pus5Wrapper { pus_5_handler }
}

pub struct Pus5Wrapper<TcInMemConverter: EcssTcInMemConverter> {
    pub pus_5_handler: PusService5EventHandler<TcInMemConverter>,
}

impl<TcInMemConverter: EcssTcInMemConverter> Pus5Wrapper<TcInMemConverter> {
    pub fn handle_next_packet(&mut self) -> bool {
        match self.pus_5_handler.handle_one_tc() {
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
