use log::{info, warn};
use satrs_core::params::Params;
use satrs_core::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs_core::pus::test::PusService17TestHandler;
use satrs_core::pus::verification::{FailParams, VerificationReporterWithSender};
use satrs_core::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInVecConverter, MpscTcReceiver, MpscTmAsVecSender,
    MpscTmInStoreSender, PusPacketHandlerResult, PusServiceHelper,
};
use satrs_core::spacepackets::ecss::tc::PusTcReader;
use satrs_core::spacepackets::ecss::PusPacket;
use satrs_core::spacepackets::time::cds::TimeProvider;
use satrs_core::spacepackets::time::TimeWriter;
use satrs_core::tmtc::tm_helper::SharedTmStore;
use satrs_core::ChannelId;
use satrs_core::{events::EventU32, pus::EcssTcInSharedStoreConverter};
use satrs_example::config::{tmtc_err, TcReceiverId, TmSenderId, PUS_APID, TEST_EVENT};
use std::sync::mpsc::{self, Sender};

pub fn create_test_service_static(
    shared_tm_store: SharedTmStore,
    tm_funnel_tx: mpsc::Sender<StoreAddr>,
    verif_reporter: VerificationReporterWithSender,
    tc_pool: SharedStaticMemoryPool,
    event_sender: mpsc::Sender<(EventU32, Option<Params>)>,
    pus_test_rx: mpsc::Receiver<EcssTcAndToken>,
) -> Service17CustomWrapper<EcssTcInSharedStoreConverter> {
    let test_srv_tm_sender = MpscTmInStoreSender::new(
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
        Box::new(test_srv_receiver),
        Box::new(test_srv_tm_sender),
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
    verif_reporter: VerificationReporterWithSender,
    event_sender: mpsc::Sender<(EventU32, Option<Params>)>,
    pus_test_rx: mpsc::Receiver<EcssTcAndToken>,
) -> Service17CustomWrapper<EcssTcInVecConverter> {
    let test_srv_tm_sender = MpscTmAsVecSender::new(
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
        Box::new(test_srv_receiver),
        Box::new(test_srv_tm_sender),
        PUS_APID,
        verif_reporter.clone(),
        EcssTcInVecConverter::default(),
    ));
    Service17CustomWrapper {
        pus17_handler,
        test_srv_event_sender: event_sender,
    }
}

pub struct Service17CustomWrapper<TcInMemConverter: EcssTcInMemConverter> {
    pub pus17_handler: PusService17TestHandler<TcInMemConverter>,
    pub test_srv_event_sender: Sender<(EventU32, Option<Params>)>,
}

impl<TcInMemConverter: EcssTcInMemConverter> Service17CustomWrapper<TcInMemConverter> {
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
                let time_stamper = TimeProvider::from_now_with_u16_days().unwrap();
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
                        .get_mut()
                        .start_success(token, Some(&stamp_buf))
                        .expect("Error sending start success");
                    self.pus17_handler
                        .service_helper
                        .common
                        .verification_handler
                        .get_mut()
                        .completion_success(start_token, Some(&stamp_buf))
                        .expect("Error sending completion success");
                } else {
                    let fail_data = [tc.subservice()];
                    self.pus17_handler
                        .service_helper
                        .common
                        .verification_handler
                        .get_mut()
                        .start_failure(
                            token,
                            FailParams::new(
                                Some(&stamp_buf),
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                Some(&fail_data),
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
