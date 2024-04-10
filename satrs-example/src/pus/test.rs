use crate::pus::create_verification_reporter;
use log::{info, warn};
use satrs::event_man::{EventMessage, EventMessageU32};
use satrs::pool::SharedStaticMemoryPool;
use satrs::pus::test::PusService17TestHandler;
use satrs::pus::verification::{FailParams, VerificationReporter, VerificationReportingProvider};
use satrs::pus::EcssTcInSharedStoreConverter;
use satrs::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInVecConverter, EcssTmSenderCore, MpscTcReceiver,
    MpscTmAsVecSender, MpscTmInSharedPoolSenderBounded, PusPacketHandlerResult, PusServiceHelper,
    PusTmAsVec, PusTmInPool, TmInSharedPoolSender,
};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusPacket;
use satrs::spacepackets::time::cds::CdsTime;
use satrs::spacepackets::time::TimeWriter;
use satrs_example::config::components::PUS_TEST_SERVICE;
use satrs_example::config::{tmtc_err, TEST_EVENT};
use std::sync::mpsc;

use super::HandlingStatus;

pub fn create_test_service_static(
    tm_sender: TmInSharedPoolSender<mpsc::SyncSender<PusTmInPool>>,
    tc_pool: SharedStaticMemoryPool,
    event_sender: mpsc::Sender<EventMessageU32>,
    pus_test_rx: mpsc::Receiver<EcssTcAndToken>,
) -> TestCustomServiceWrapper<MpscTmInSharedPoolSenderBounded, EcssTcInSharedStoreConverter> {
    let pus17_handler = PusService17TestHandler::new(PusServiceHelper::new(
        PUS_TEST_SERVICE.id(),
        pus_test_rx,
        tm_sender,
        create_verification_reporter(PUS_TEST_SERVICE.id(), PUS_TEST_SERVICE.apid),
        EcssTcInSharedStoreConverter::new(tc_pool, 2048),
    ));
    TestCustomServiceWrapper {
        handler: pus17_handler,
        test_srv_event_sender: event_sender,
    }
}

pub fn create_test_service_dynamic(
    tm_funnel_tx: mpsc::Sender<PusTmAsVec>,
    event_sender: mpsc::Sender<EventMessageU32>,
    pus_test_rx: mpsc::Receiver<EcssTcAndToken>,
) -> TestCustomServiceWrapper<MpscTmAsVecSender, EcssTcInVecConverter> {
    let pus17_handler = PusService17TestHandler::new(PusServiceHelper::new(
        PUS_TEST_SERVICE.id(),
        pus_test_rx,
        tm_funnel_tx,
        create_verification_reporter(PUS_TEST_SERVICE.id(), PUS_TEST_SERVICE.apid),
        EcssTcInVecConverter::default(),
    ));
    TestCustomServiceWrapper {
        handler: pus17_handler,
        test_srv_event_sender: event_sender,
    }
}

pub struct TestCustomServiceWrapper<
    TmSender: EcssTmSenderCore,
    TcInMemConverter: EcssTcInMemConverter,
> {
    pub handler:
        PusService17TestHandler<MpscTcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    pub test_srv_event_sender: mpsc::Sender<EventMessageU32>,
}

impl<TmSender: EcssTmSenderCore, TcInMemConverter: EcssTcInMemConverter>
    TestCustomServiceWrapper<TmSender, TcInMemConverter>
{
    pub fn poll_and_handle_next_packet(&mut self, time_stamp: &[u8]) -> HandlingStatus {
        let res = self.handler.poll_and_handle_next_tc(time_stamp);
        if res.is_err() {
            warn!("PUS17 handler failed with error {:?}", res.unwrap_err());
            return HandlingStatus::HandledOne;
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
                    self.handler
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
                        .send(EventMessage::new(PUS_TEST_SERVICE.id(), TEST_EVENT.into()))
                        .expect("Sending test event failed");
                    let start_token = self
                        .handler
                        .service_helper
                        .verif_reporter()
                        .start_success(self.handler.service_helper.tm_sender(), token, &stamp_buf)
                        .expect("Error sending start success");
                    self.handler
                        .service_helper
                        .verif_reporter()
                        .completion_success(
                            self.handler.service_helper.tm_sender(),
                            start_token,
                            &stamp_buf,
                        )
                        .expect("Error sending completion success");
                } else {
                    let fail_data = [tc.subservice()];
                    self.handler
                        .service_helper
                        .verif_reporter()
                        .start_failure(
                            self.handler.service_helper.tm_sender(),
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
            PusPacketHandlerResult::Empty => return HandlingStatus::Empty,
        }
        HandlingStatus::HandledOne
    }
}
