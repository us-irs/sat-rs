use crate::events::EventU32;
use crate::pus::event_man::{EventRequest, EventRequestWithToken};
use crate::pus::verification::TcStateToken;
use crate::pus::{PartialPusHandlingError, PusPacketHandlerResult, PusPacketHandlingError};
use spacepackets::ecss::event::Subservice;
use spacepackets::ecss::PusPacket;
use std::sync::mpsc::Sender;

use super::verification::VerificationReportingProvider;
use super::{EcssTcInMemConverter, PusServiceBase, PusServiceHelper};

pub struct PusService5EventHandler<
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    pub service_helper: PusServiceHelper<TcInMemConverter, VerificationReporter>,
    event_request_tx: Sender<EventRequestWithToken>,
}

impl<
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > PusService5EventHandler<TcInMemConverter, VerificationReporter>
{
    pub fn new(
        service_helper: PusServiceHelper<TcInMemConverter, VerificationReporter>,
        event_request_tx: Sender<EventRequestWithToken>,
    ) -> Self {
        Self {
            service_helper,
            event_request_tx,
        }
    }

    pub fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        let possible_packet = self.service_helper.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(PusPacketHandlerResult::Empty);
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        let tc = self
            .service_helper
            .tc_in_mem_converter
            .convert_ecss_tc_in_memory_to_reader(&ecss_tc_and_token.tc_in_memory)?;
        let subservice = tc.subservice();
        let srv = Subservice::try_from(subservice);
        if srv.is_err() {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                ecss_tc_and_token.token,
            ));
        }
        let handle_enable_disable_request = |enable: bool, stamp: [u8; 7]| {
            if tc.user_data().len() < 4 {
                return Err(PusPacketHandlingError::NotEnoughAppData {
                    expected: 4,
                    found: tc.user_data().len(),
                });
            }
            let user_data = tc.user_data();
            let event_u32 = EventU32::from(u32::from_be_bytes(user_data[0..4].try_into().unwrap()));
            let start_token = self
                .service_helper
                .common
                .verification_handler
                .start_success(ecss_tc_and_token.token, &stamp)
                .map_err(|_| PartialPusHandlingError::Verification);
            let partial_error = start_token.clone().err();
            let mut token: TcStateToken = ecss_tc_and_token.token.into();
            if let Ok(start_token) = start_token {
                token = start_token.into();
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
                    PusPacketHandlingError::Other("Forwarding event request failed".into())
                })?;
            if let Some(partial_error) = partial_error {
                return Ok(PusPacketHandlerResult::RequestHandledPartialSuccess(
                    partial_error,
                ));
            }
            Ok(PusPacketHandlerResult::RequestHandled)
        };
        let mut partial_error = None;
        let time_stamp = PusServiceBase::<VerificationReporter>::get_current_cds_short_timestamp(
            &mut partial_error,
        );
        match srv.unwrap() {
            Subservice::TmInfoReport
            | Subservice::TmLowSeverityReport
            | Subservice::TmMediumSeverityReport
            | Subservice::TmHighSeverityReport => {
                return Err(PusPacketHandlingError::InvalidSubservice(tc.subservice()))
            }
            Subservice::TcEnableEventGeneration => {
                handle_enable_disable_request(true, time_stamp)?;
            }
            Subservice::TcDisableEventGeneration => {
                handle_enable_disable_request(false, time_stamp)?;
            }
            Subservice::TcReportDisabledList | Subservice::TmDisabledEventsReport => {
                return Ok(PusPacketHandlerResult::SubserviceNotImplemented(
                    subservice,
                    ecss_tc_and_token.token,
                ));
            }
        }

        Ok(PusPacketHandlerResult::RequestHandled)
    }
}

#[cfg(test)]
mod tests {
    use delegate::delegate;
    use spacepackets::ecss::event::Subservice;
    use spacepackets::util::UnsignedEnum;
    use spacepackets::{
        ecss::{
            tc::{PusTcCreator, PusTcSecondaryHeader},
            tm::PusTmReader,
        },
        SequenceFlags, SpHeader,
    };
    use std::sync::mpsc::{self, Sender};

    use crate::pus::event_man::EventRequest;
    use crate::pus::tests::SimplePusPacketHandler;
    use crate::pus::verification::{
        RequestId, VerificationReporterWithSharedPoolMpscBoundedSender,
    };
    use crate::{
        events::EventU32,
        pus::{
            event_man::EventRequestWithToken,
            tests::{PusServiceHandlerWithSharedStoreCommon, PusTestHarness, TEST_APID},
            verification::{TcStateAccepted, VerificationToken},
            EcssTcInSharedStoreConverter, PusPacketHandlerResult, PusPacketHandlingError,
        },
    };

    use super::PusService5EventHandler;

    const TEST_EVENT_0: EventU32 = EventU32::const_new(crate::events::Severity::INFO, 5, 25);

    struct Pus5HandlerWithStoreTester {
        common: PusServiceHandlerWithSharedStoreCommon,
        handler: PusService5EventHandler<
            EcssTcInSharedStoreConverter,
            VerificationReporterWithSharedPoolMpscBoundedSender,
        >,
    }

    impl Pus5HandlerWithStoreTester {
        pub fn new(event_request_tx: Sender<EventRequestWithToken>) -> Self {
            let (common, srv_handler) = PusServiceHandlerWithSharedStoreCommon::new();
            Self {
                common,
                handler: PusService5EventHandler::new(srv_handler, event_request_tx),
            }
        }
    }

    impl PusTestHarness for Pus5HandlerWithStoreTester {
        delegate! {
            to self.common {
                fn send_tc(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted>;
                fn read_next_tm(&mut self) -> PusTmReader<'_>;
                fn check_no_tm_available(&self) -> bool;
                fn check_next_verification_tm(&self, subservice: u8, expected_request_id: RequestId);
            }

        }
    }

    impl SimplePusPacketHandler for Pus5HandlerWithStoreTester {
        delegate! {
            to self.handler {
                fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError>;
            }
        }
    }

    fn event_test(
        test_harness: &mut (impl PusTestHarness + SimplePusPacketHandler),
        subservice: Subservice,
        expected_event_req: EventRequest,
        event_req_receiver: mpsc::Receiver<EventRequestWithToken>,
    ) {
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(5, subservice as u8);
        let mut app_data = [0; 4];
        TEST_EVENT_0
            .write_to_be_bytes(&mut app_data)
            .expect("writing test event failed");
        let ping_tc = PusTcCreator::new(&mut sp_header, sec_header, &app_data, true);
        let token = test_harness.send_tc(&ping_tc);
        let request_id = token.req_id();
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
        if let PusPacketHandlerResult::Empty = result {
        } else {
            panic!("unexpected result type {result:?}")
        }
    }

    #[test]
    fn test_sending_custom_subservice() {
        let (event_request_tx, _) = mpsc::channel();
        let mut test_harness = Pus5HandlerWithStoreTester::new(event_request_tx);
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(5, 200);
        let ping_tc = PusTcCreator::new_no_app_data(&mut sp_header, sec_header, true);
        test_harness.send_tc(&ping_tc);
        let result = test_harness.handle_one_tc();
        assert!(result.is_ok());
        let result = result.unwrap();
        if let PusPacketHandlerResult::CustomSubservice(subservice, _) = result {
            assert_eq!(subservice, 200);
        } else {
            panic!("unexpected result type {result:?}")
        }
    }

    #[test]
    fn test_sending_invalid_app_data() {
        let (event_request_tx, _) = mpsc::channel();
        let mut test_harness = Pus5HandlerWithStoreTester::new(event_request_tx);
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header =
            PusTcSecondaryHeader::new_simple(5, Subservice::TcEnableEventGeneration as u8);
        let ping_tc = PusTcCreator::new(&mut sp_header, sec_header, &[0, 1, 2], true);
        test_harness.send_tc(&ping_tc);
        let result = test_harness.handle_one_tc();
        assert!(result.is_err());
        let result = result.unwrap_err();
        if let PusPacketHandlingError::NotEnoughAppData { expected, found } = result {
            assert_eq!(expected, 4);
            assert_eq!(found, 3);
        } else {
            panic!("unexpected result type {result:?}")
        }
    }
}
