use log::{info, warn};
use satrs::params::Params;
use satrs::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs::pus::test::PusService17TestHandler;
use satrs::pus::verification::{FailParams, VerificationReportingProvider};
use satrs::pus::verification::{
    VerificationReporterWithSharedPoolMpscBoundedSender, VerificationReporterWithVecMpscSender,
};
use satrs::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInVecConverter, EcssTcReceiverCore,
    EcssTmSenderCore, MpscTcReceiver, PusPacketHandlerResult, PusServiceHelper,
    TmAsVecSenderWithId, TmAsVecSenderWithMpsc, TmInSharedPoolSenderWithBoundedMpsc,
    TmInSharedPoolSenderWithId,
};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusPacket;
use satrs::spacepackets::time::cds::CdsTime;
use satrs::spacepackets::time::TimeWriter;
use satrs::tmtc::tm_helper::SharedTmPool;
use satrs::ChannelId;
use satrs::{events::EventU32, pus::EcssTcInSharedStoreConverter};
use satrs_example::config::{tmtc_err, TcReceiverId, TmSenderId, PUS_APID, TEST_EVENT};
use std::sync::mpsc::{self, Sender};

pub fn create_test_service_static(
    shared_tm_store: SharedTmPool,
    tm_funnel_tx: mpsc::SyncSender<StoreAddr>,
    verif_reporter: VerificationReporterWithSharedPoolMpscBoundedSender,
    tc_pool: SharedStaticMemoryPool,
    event_sender: mpsc::Sender<(EventU32, Option<Params>)>,
    pus_test_rx: mpsc::Receiver<EcssTcAndToken>,
) -> Service17CustomWrapper<
    MpscTcReceiver,
    TmInSharedPoolSenderWithBoundedMpsc,
    EcssTcInSharedStoreConverter,
    VerificationReporterWithSharedPoolMpscBoundedSender,
> {
    let test_srv_tm_sender = TmInSharedPoolSenderWithId::new(
        TmSenderId::PusTest as ChannelId,
        "PUS_17_TM_SENDER",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    );
    let test_srv_receiver = MpscTcReceiver::new(
        TcReceiverId::PusTest as ChannelId,
        "PUS_17_TC_RECV",
        pus_test_rx,
    );
    let pus17_handler = PusService17TestHandler::new(PusServiceHelper::new(
        test_srv_receiver,
        test_srv_tm_sender,
        PUS_APID,
        verif_reporter.clone(),
        EcssTcInSharedStoreConverter::new(tc_pool, 2048),
    ));
    Service17CustomWrapper {
        pus17_handler,
        test_srv_event_sender: event_sender,
    }
}

pub fn create_test_service_dynamic(
    tm_funnel_tx: mpsc::Sender<Vec<u8>>,
    verif_reporter: VerificationReporterWithVecMpscSender,
    event_sender: mpsc::Sender<(EventU32, Option<Params>)>,
    pus_test_rx: mpsc::Receiver<EcssTcAndToken>,
) -> Service17CustomWrapper<
    MpscTcReceiver,
    TmAsVecSenderWithMpsc,
    EcssTcInVecConverter,
    VerificationReporterWithVecMpscSender,
> {
    let test_srv_tm_sender = TmAsVecSenderWithId::new(
        TmSenderId::PusTest as ChannelId,
        "PUS_17_TM_SENDER",
        tm_funnel_tx.clone(),
    );
    let test_srv_receiver = MpscTcReceiver::new(
        TcReceiverId::PusTest as ChannelId,
        "PUS_17_TC_RECV",
        pus_test_rx,
    );
    let pus17_handler = PusService17TestHandler::new(PusServiceHelper::new(
        test_srv_receiver,
        test_srv_tm_sender,
        PUS_APID,
        verif_reporter.clone(),
        EcssTcInVecConverter::default(),
    ));
    Service17CustomWrapper {
        pus17_handler,
        test_srv_event_sender: event_sender,
    }
}

pub struct Service17CustomWrapper<
    TcReceiver: EcssTcReceiverCore,
    TmSender: EcssTmSenderCore,
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    pub pus17_handler:
        PusService17TestHandler<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    pub test_srv_event_sender: Sender<(EventU32, Option<Params>)>,
}

impl<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > Service17CustomWrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>
{
    pub fn handle_next_packet(&mut self) -> bool {
        let res = self.pus17_handler.handle_one_tc();
        if res.is_err() {
            warn!("PUS17 handler failed with error {:?}", res.unwrap_err());
            return true;
        }
        match res.unwrap() {
            PusPacketHandlerResult::RequestHandled => {
                info!("Received PUS ping command TC[17,1]");
                info!("Sent ping reply PUS TM[17,2]");
            }
            PusPacketHandlerResult::RequestHandledPartialSuccess(partial_err) => {
                warn!(
                    "Handled PUS ping command with partial success: {:?}",
                    partial_err
                );
            }
            PusPacketHandlerResult::SubserviceNotImplemented(subservice, _) => {
                warn!("PUS17: Subservice {subservice} not implemented")
            }
            PusPacketHandlerResult::CustomSubservice(subservice, token) => {
                let (tc, _) = PusTcReader::new(
                    self.pus17_handler
                        .service_helper
                        .tc_in_mem_converter
                        .tc_slice_raw(),
                )
                .unwrap();
                let time_stamper = CdsTime::now_with_u16_days().unwrap();
                let mut stamp_buf: [u8; 7] = [0; 7];
                time_stamper.write_to_bytes(&mut stamp_buf).unwrap();
                if subservice == 128 {
                    info!("Generating test event");
                    self.test_srv_event_sender
                        .send((TEST_EVENT.into(), None))
                        .expect("Sending test event failed");
                    let start_token = self
                        .pus17_handler
                        .service_helper
                        .common
                        .verification_handler
                        .start_success(token, &stamp_buf)
                        .expect("Error sending start success");
                    self.pus17_handler
                        .service_helper
                        .common
                        .verification_handler
                        .completion_success(start_token, &stamp_buf)
                        .expect("Error sending completion success");
                } else {
                    let fail_data = [tc.subservice()];
                    self.pus17_handler
                        .service_helper
                        .common
                        .verification_handler
                        .start_failure(
                            token,
                            FailParams::new(
                                &stamp_buf,
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                &fail_data,
                            ),
                        )
                        .expect("Sending start failure verification failed");
                }
            }
            PusPacketHandlerResult::Empty => {
                return true;
            }
        }
        false
    }
}
