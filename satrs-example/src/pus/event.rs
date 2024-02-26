use std::sync::mpsc;

use log::{error, warn};
use satrs::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs::pus::event_man::EventRequestWithToken;
use satrs::pus::event_srv::PusService5EventHandler;
use satrs::pus::verification::std_mod::{
    VerificationReporterWithSharedPoolMpscBoundedSender, VerificationReporterWithVecMpscSender,
};
use satrs::pus::verification::VerificationReportingProvider;
use satrs::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter,
    MpscTcReceiver, PusPacketHandlerResult, PusServiceHelper, TmAsVecSenderWithId,
    TmInSharedPoolSenderWithId,
};
use satrs::tmtc::tm_helper::SharedTmPool;
use satrs::ChannelId;
use satrs_example::config::{TcReceiverId, TmSenderId, PUS_APID};

pub fn create_event_service_static(
    shared_tm_store: SharedTmPool,
    tm_funnel_tx: mpsc::SyncSender<StoreAddr>,
    verif_reporter: VerificationReporterWithSharedPoolMpscBoundedSender,
    tc_pool: SharedStaticMemoryPool,
    pus_event_rx: mpsc::Receiver<EcssTcAndToken>,
    event_request_tx: mpsc::Sender<EventRequestWithToken>,
) -> Pus5Wrapper<EcssTcInSharedStoreConverter, VerificationReporterWithSharedPoolMpscBoundedSender>
{
    let event_srv_tm_sender = TmInSharedPoolSenderWithId::new(
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
    verif_reporter: VerificationReporterWithVecMpscSender,
    pus_event_rx: mpsc::Receiver<EcssTcAndToken>,
    event_request_tx: mpsc::Sender<EventRequestWithToken>,
) -> Pus5Wrapper<EcssTcInVecConverter, VerificationReporterWithVecMpscSender> {
    let event_srv_tm_sender = TmAsVecSenderWithId::new(
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

pub struct Pus5Wrapper<
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    pub pus_5_handler: PusService5EventHandler<TcInMemConverter, VerificationReporter>,
}

impl<
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > Pus5Wrapper<TcInMemConverter, VerificationReporter>
{
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
