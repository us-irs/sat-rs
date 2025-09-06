use crate::events::EventU32;
use crate::pus::event_man::{EventRequest, EventRequestWithToken};
use crate::pus::verification::TcStateToken;
use crate::pus::{DirectPusPacketHandlerResult, PartialPusHandlingError, PusPacketHandlingError};
use crate::queue::GenericSendError;
use spacepackets::ecss::PusPacket;
use spacepackets::ecss::event::Subservice;
use std::sync::mpsc::Sender;

use super::verification::VerificationReportingProvider;
use super::{
    CacheAndReadRawEcssTc, EcssTcReceiver, EcssTmSender, GenericConversionError,
    GenericRoutingError, HandlingStatus, PusServiceHelper,
};

pub struct PusEventServiceHandler<
    TcReceiver: EcssTcReceiver,
    TmSender: EcssTmSender,
    TcInMemConverter: CacheAndReadRawEcssTc,
    VerificationReporter: VerificationReportingProvider,
> {
    pub service_helper:
        PusServiceHelper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    event_request_tx: Sender<EventRequestWithToken>,
}

impl<
    TcReceiver: EcssTcReceiver,
    TmSender: EcssTmSender,
    TcInMemConverter: CacheAndReadRawEcssTc,
    VerificationReporter: VerificationReportingProvider,
> PusEventServiceHandler<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>
{
    pub fn new(
        service_helper: PusServiceHelper<
            TcReceiver,
            TmSender,
            TcInMemConverter,
            VerificationReporter,
        >,
        event_request_tx: Sender<EventRequestWithToken>,
    ) -> Self {
        Self {
            service_helper,
            event_request_tx,
        }
    }

    pub fn poll_and_handle_next_tc<ErrorCb: FnMut(&PartialPusHandlingError)>(
        &mut self,
        mut error_callback: ErrorCb,
        time_stamp: &[u8],
    ) -> Result<DirectPusPacketHandlerResult, PusPacketHandlingError> {
        let possible_packet = self.service_helper.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(HandlingStatus::Empty.into());
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        self.service_helper
            .tc_in_mem_converter_mut()
            .cache(&ecss_tc_and_token.tc_in_memory)?;
        let tc = self.service_helper.tc_in_mem_converter().convert()?;
        let subservice = tc.subservice();
        let srv = Subservice::try_from(subservice);
        if srv.is_err() {
            return Ok(DirectPusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                ecss_tc_and_token.token,
            ));
        }
        let mut handle_enable_disable_request =
            |enable: bool| -> Result<DirectPusPacketHandlerResult, PusPacketHandlingError> {
                if tc.user_data().len() < 4 {
                    return Err(GenericConversionError::NotEnoughAppData {
                        expected: 4,
                        found: tc.user_data().len(),
                    }
                    .into());
                }
                let user_data = tc.user_data();
                let event_u32 =
                    EventU32::from(u32::from_be_bytes(user_data[0..4].try_into().unwrap()));
                let mut token: TcStateToken = ecss_tc_and_token.token.into();
                match self.service_helper.common.verif_reporter.start_success(
                    &self.service_helper.common.tm_sender,
                    ecss_tc_and_token.token,
                    time_stamp,
                ) {
                    Ok(start_token) => {
                        token = start_token.into();
                    }
                    Err(e) => {
                        error_callback(&PartialPusHandlingError::Verification(e));
                    }
                }

                let event_req_with_token = if enable {
                    EventRequestWithToken {
                        request: EventRequest::Enable(event_u32),
                        token,
                    }
                } else {
                    EventRequestWithToken {
                        request: EventRequest::Disable(event_u32),
                        token,
                    }
                };
                self.event_request_tx
                    .send(event_req_with_token)
                    .map_err(|_| {
                        PusPacketHandlingError::RequestRouting(GenericRoutingError::Send(
                            GenericSendError::RxDisconnected,
                        ))
                    })?;
                Ok(HandlingStatus::HandledOne.into())
            };

        match srv.unwrap() {
            Subservice::TmInfoReport
            | Subservice::TmLowSeverityReport
            | Subservice::TmMediumSeverityReport
            | Subservice::TmHighSeverityReport => {
                return Err(PusPacketHandlingError::RequestConversion(
                    GenericConversionError::WrongService(tc.subservice()),
                ));
            }
            Subservice::TcEnableEventGeneration => {
                handle_enable_disable_request(true)?;
            }
            Subservice::TcDisableEventGeneration => {
                handle_enable_disable_request(false)?;
            }
            Subservice::TcReportDisabledList | Subservice::TmDisabledEventsReport => {
                return Ok(DirectPusPacketHandlerResult::SubserviceNotImplemented(
                    subservice,
                    ecss_tc_and_token.token,
                ));
            }
        }

        Ok(HandlingStatus::HandledOne.into())
    }
}

#[cfg(test)]
mod tests {
    use delegate::delegate;
    use spacepackets::ecss::event::Subservice;
    use spacepackets::time::{TimeWriter, cds};
    use spacepackets::util::UnsignedEnum;
    use spacepackets::{
        SpHeader,
        ecss::{
            tc::{PusTcCreator, PusTcSecondaryHeader},
            tm::PusTmReader,
        },
    };
    use std::sync::mpsc::{self, Sender};

    use crate::pus::event_man::EventRequest;
    use crate::pus::test_util::{PusTestHarness, SimplePusPacketHandler, TEST_APID};
    use crate::pus::verification::{
        RequestId, VerificationReporter, VerificationReportingProvider,
    };
    use crate::pus::{GenericConversionError, HandlingStatus, MpscTcReceiver};
    use crate::tmtc::PacketSenderWithSharedPool;
    use crate::{
        events::EventU32,
        pus::{
            DirectPusPacketHandlerResult, EcssTcInSharedPoolCacher, PusPacketHandlingError,
            event_man::EventRequestWithToken,
            tests::PusServiceHandlerWithSharedStoreCommon,
            verification::{TcStateAccepted, VerificationToken},
        },
    };

    use super::PusEventServiceHandler;

    const TEST_EVENT_0: EventU32 = EventU32::new(crate::events::Severity::Info, 5, 25);

    struct Pus5HandlerWithStoreTester {
        common: PusServiceHandlerWithSharedStoreCommon,
        handler: PusEventServiceHandler<
            MpscTcReceiver,
            PacketSenderWithSharedPool,
            EcssTcInSharedPoolCacher,
            VerificationReporter,
        >,
    }

    impl Pus5HandlerWithStoreTester {
        pub fn new(event_request_tx: Sender<EventRequestWithToken>) -> Self {
            let (common, srv_handler) = PusServiceHandlerWithSharedStoreCommon::new(0);
            Self {
                common,
                handler: PusEventServiceHandler::new(srv_handler, event_request_tx),
            }
        }
    }

    impl PusTestHarness for Pus5HandlerWithStoreTester {
        fn start_verification(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted> {
            let init_token = self
                .handler
                .service_helper
                .verif_reporter_mut()
                .start_verification(tc);
            self.handler
                .service_helper
                .verif_reporter()
                .acceptance_success(self.handler.service_helper.tm_sender(), init_token, &[0; 7])
                .expect("acceptance success failure")
        }

        fn send_tc(&self, token: &VerificationToken<TcStateAccepted>, tc: &PusTcCreator) {
            self.common
                .send_tc(self.handler.service_helper.id(), token, tc);
        }

        delegate! {
            to self.common {
                fn read_next_tm(&mut self) -> PusTmReader<'_>;
                fn check_no_tm_available(&self) -> bool;
                fn check_next_verification_tm(&self, subservice: u8, expected_request_id: RequestId);
            }

        }
    }

    impl SimplePusPacketHandler for Pus5HandlerWithStoreTester {
        fn handle_one_tc(
            &mut self,
        ) -> Result<DirectPusPacketHandlerResult, PusPacketHandlingError> {
            let time_stamp = cds::CdsTime::new_with_u16_days(0, 0).to_vec().unwrap();
            self.handler.poll_and_handle_next_tc(|_| {}, &time_stamp)
        }
    }

    fn event_test(
        test_harness: &mut (impl PusTestHarness + SimplePusPacketHandler),
        subservice: Subservice,
        expected_event_req: EventRequest,
        event_req_receiver: mpsc::Receiver<EventRequestWithToken>,
    ) {
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header = PusTcSecondaryHeader::new_simple(5, subservice as u8);
        let mut app_data = [0; 4];
        TEST_EVENT_0
            .write_to_be_bytes(&mut app_data)
            .expect("writing test event failed");
        let ping_tc = PusTcCreator::new(sp_header, sec_header, &app_data, true);
        let token = test_harness.start_verification(&ping_tc);
        test_harness.send_tc(&token, &ping_tc);
        let request_id = token.request_id();
        test_harness.handle_one_tc().unwrap();
        test_harness.check_next_verification_tm(1, request_id);
        test_harness.check_next_verification_tm(3, request_id);
        // Completion TM is not generated for us.
        assert!(test_harness.check_no_tm_available());
        let event_request = event_req_receiver
            .try_recv()
            .expect("no event request received");
        assert_eq!(expected_event_req, event_request.request);
    }

    #[test]
    fn test_enabling_event_reporting() {
        let (event_request_tx, event_request_rx) = mpsc::channel();
        let mut test_harness = Pus5HandlerWithStoreTester::new(event_request_tx);
        event_test(
            &mut test_harness,
            Subservice::TcEnableEventGeneration,
            EventRequest::Enable(TEST_EVENT_0),
            event_request_rx,
        );
    }

    #[test]
    fn test_disabling_event_reporting() {
        let (event_request_tx, event_request_rx) = mpsc::channel();
        let mut test_harness = Pus5HandlerWithStoreTester::new(event_request_tx);
        event_test(
            &mut test_harness,
            Subservice::TcDisableEventGeneration,
            EventRequest::Disable(TEST_EVENT_0),
            event_request_rx,
        );
    }

    #[test]
    fn test_empty_tc_queue() {
        let (event_request_tx, _) = mpsc::channel();
        let mut test_harness = Pus5HandlerWithStoreTester::new(event_request_tx);
        let result = test_harness.handle_one_tc();
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(
            matches!(
                result,
                DirectPusPacketHandlerResult::Handled(HandlingStatus::Empty)
            ),
            "unexpected result type {result:?}"
        )
    }

    #[test]
    fn test_sending_custom_subservice() {
        let (event_request_tx, _) = mpsc::channel();
        let mut test_harness = Pus5HandlerWithStoreTester::new(event_request_tx);
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header = PusTcSecondaryHeader::new_simple(5, 200);
        let ping_tc = PusTcCreator::new_no_app_data(sp_header, sec_header, true);
        let token = test_harness.start_verification(&ping_tc);
        test_harness.send_tc(&token, &ping_tc);
        let result = test_harness.handle_one_tc();
        assert!(result.is_ok());
        let result = result.unwrap();
        if let DirectPusPacketHandlerResult::CustomSubservice(subservice, _) = result {
            assert_eq!(subservice, 200);
        } else {
            panic!("unexpected result type {result:?}")
        }
    }

    #[test]
    fn test_sending_invalid_app_data() {
        let (event_request_tx, _) = mpsc::channel();
        let mut test_harness = Pus5HandlerWithStoreTester::new(event_request_tx);
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header =
            PusTcSecondaryHeader::new_simple(5, Subservice::TcEnableEventGeneration as u8);
        let ping_tc = PusTcCreator::new(sp_header, sec_header, &[0, 1, 2], true);
        let token = test_harness.start_verification(&ping_tc);
        test_harness.send_tc(&token, &ping_tc);
        let result = test_harness.handle_one_tc();
        assert!(result.is_err());
        let result = result.unwrap_err();
        if let PusPacketHandlingError::RequestConversion(
            GenericConversionError::NotEnoughAppData { expected, found },
        ) = result
        {
            assert_eq!(expected, 4);
            assert_eq!(found, 3);
        } else {
            panic!("unexpected result type {result:?}")
        }
    }
}
