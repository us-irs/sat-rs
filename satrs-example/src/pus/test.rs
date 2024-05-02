use crate::pus::create_verification_reporter;
use log::info;
use satrs::event_man::{EventMessage, EventMessageU32};
use satrs::pool::SharedStaticMemoryPool;
use satrs::pus::test::PusService17TestHandler;
use satrs::pus::verification::{FailParams, VerificationReporter, VerificationReportingProvider};
use satrs::pus::{
    DirectPusPacketHandlerResult, EcssTcAndToken, EcssTcInMemConverter, EcssTcInVecConverter,
    EcssTmSender, MpscTcReceiver, MpscTmAsVecSender, PusServiceHelper,
};
use satrs::pus::{EcssTcInSharedStoreConverter, PartialPusHandlingError};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::{PusPacket, PusServiceId};
use satrs::tmtc::{PacketAsVec, PacketSenderWithSharedPool};
use satrs_example::config::components::PUS_TEST_SERVICE;
use satrs_example::config::{tmtc_err, TEST_EVENT};
use std::sync::mpsc;

use super::{DirectPusService, HandlingStatus};

pub fn create_test_service_static(
    tm_sender: PacketSenderWithSharedPool,
    tc_pool: SharedStaticMemoryPool,
    event_sender: mpsc::SyncSender<EventMessageU32>,
    pus_test_rx: mpsc::Receiver<EcssTcAndToken>,
) -> TestCustomServiceWrapper<PacketSenderWithSharedPool, EcssTcInSharedStoreConverter> {
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
    tm_funnel_tx: mpsc::Sender<PacketAsVec>,
    event_sender: mpsc::SyncSender<EventMessageU32>,
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

pub struct TestCustomServiceWrapper<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter>
{
    pub handler:
        PusService17TestHandler<MpscTcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    pub test_srv_event_sender: mpsc::SyncSender<EventMessageU32>,
}

impl<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter> DirectPusService
    for TestCustomServiceWrapper<TmSender, TcInMemConverter>
{
    const SERVICE_ID: u8 = PusServiceId::Test as u8;

    const SERVICE_STR: &'static str = "test";
}

impl<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter>
    TestCustomServiceWrapper<TmSender, TcInMemConverter>
{
    pub fn poll_and_handle_next_tc(&mut self, timestamp: &[u8]) -> HandlingStatus {
        let error_handler = |partial_error: &PartialPusHandlingError| {
            log::warn!(
                "PUS {}({}) partial error: {:?}",
                Self::SERVICE_ID,
                Self::SERVICE_STR,
                partial_error
            );
        };
        let res = self
            .handler
            .poll_and_handle_next_tc(error_handler, timestamp);
        if let Err(e) = res {
            log::warn!(
                "PUS {}({}) error: {:?}",
                Self::SERVICE_ID,
                Self::SERVICE_STR,
                e
            );
            return HandlingStatus::HandledOne;
        }
        match res.unwrap() {
            DirectPusPacketHandlerResult::Handled(handling_status) => {
                if handling_status == HandlingStatus::HandledOne {
                    info!("Received PUS ping command TC[17,1]");
                    info!("Sent ping reply PUS TM[17,2]");
                }
                return handling_status;
            }
            DirectPusPacketHandlerResult::SubserviceNotImplemented(subservice, _) => {
                log::warn!(
                    "PUS {}({}) subservice {} not implemented",
                    Self::SERVICE_ID,
                    Self::SERVICE_STR,
                    subservice
                );
            }
            DirectPusPacketHandlerResult::CustomSubservice(subservice, token) => {
                let (tc, _) = PusTcReader::new(
                    self.handler
                        .service_helper
                        .tc_in_mem_converter
                        .tc_slice_raw(),
                )
                .unwrap();
                if subservice == 128 {
                    info!("generating test event");
                    self.test_srv_event_sender
                        .send(EventMessage::new(PUS_TEST_SERVICE.id(), TEST_EVENT.into()))
                        .expect("Sending test event failed");
                    match self.handler.service_helper.verif_reporter().start_success(
                        self.handler.service_helper.tm_sender(),
                        token,
                        timestamp,
                    ) {
                        Ok(started_token) => {
                            if let Err(e) = self
                                .handler
                                .service_helper
                                .verif_reporter()
                                .completion_success(
                                    self.handler.service_helper.tm_sender(),
                                    started_token,
                                    timestamp,
                                )
                            {
                                error_handler(&PartialPusHandlingError::Verification(e));
                            }
                        }
                        Err(e) => {
                            error_handler(&PartialPusHandlingError::Verification(e));
                        }
                    }
                } else {
                    let fail_data = [tc.subservice()];
                    self.handler
                        .service_helper
                        .verif_reporter()
                        .start_failure(
                            self.handler.service_helper.tm_sender(),
                            token,
                            FailParams::new(
                                timestamp,
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                &fail_data,
                            ),
                        )
                        .expect("Sending start failure verification failed");
                }
            }
        }
        HandlingStatus::HandledOne
    }
}
