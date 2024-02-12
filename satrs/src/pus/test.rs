use crate::pus::{
    PartialPusHandlingError, PusPacketHandlerResult, PusPacketHandlingError, PusTmWrapper,
};
use spacepackets::ecss::tm::{PusTmCreator, PusTmSecondaryHeader};
use spacepackets::ecss::PusPacket;
use spacepackets::SpHeader;

use super::{EcssTcInMemConverter, PusServiceBase, PusServiceHelper};

/// This is a helper class for [std] environments to handle generic PUS 17 (test service) packets.
/// This handler only processes ping requests and generates a ping reply for them accordingly.
pub struct PusService17TestHandler<TcInMemConverter: EcssTcInMemConverter> {
    pub service_helper: PusServiceHelper<TcInMemConverter>,
}

impl<TcInMemConverter: EcssTcInMemConverter> PusService17TestHandler<TcInMemConverter> {
    pub fn new(service_helper: PusServiceHelper<TcInMemConverter>) -> Self {
        Self { service_helper }
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
        if tc.service() != 17 {
            return Err(PusPacketHandlingError::WrongService(tc.service()));
        }
        if tc.subservice() == 1 {
            let mut partial_error = None;
            let time_stamp = PusServiceBase::get_current_timestamp(&mut partial_error);
            let result = self
                .service_helper
                .common
                .verification_handler
                .get_mut()
                .start_success(ecss_tc_and_token.token, Some(&time_stamp))
                .map_err(|_| PartialPusHandlingError::Verification);
            let start_token = if let Ok(result) = result {
                Some(result)
            } else {
                partial_error = Some(result.unwrap_err());
                None
            };
            // Sequence count will be handled centrally in TM funnel.
            let mut reply_header =
                SpHeader::tm_unseg(self.service_helper.common.tm_apid, 0, 0).unwrap();
            let tc_header = PusTmSecondaryHeader::new_simple(17, 2, &time_stamp);
            let ping_reply = PusTmCreator::new(&mut reply_header, tc_header, &[], true);
            let result = self
                .service_helper
                .common
                .tm_sender
                .send_tm(PusTmWrapper::Direct(ping_reply))
                .map_err(PartialPusHandlingError::TmSend);
            if let Err(err) = result {
                partial_error = Some(err);
            }

            if let Some(start_token) = start_token {
                if self
                    .service_helper
                    .common
                    .verification_handler
                    .get_mut()
                    .completion_success(start_token, Some(&time_stamp))
                    .is_err()
                {
                    partial_error = Some(PartialPusHandlingError::Verification)
                }
            }
            if let Some(partial_error) = partial_error {
                return Ok(PusPacketHandlerResult::RequestHandledPartialSuccess(
                    partial_error,
                ));
            };
        } else {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                tc.subservice(),
                ecss_tc_and_token.token,
            ));
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}

#[cfg(test)]
mod tests {
    use crate::pus::tests::{
        PusServiceHandlerWithSharedStoreCommon, PusServiceHandlerWithVecCommon, PusTestHarness,
        SimplePusPacketHandler, TEST_APID,
    };
    use crate::pus::verification::RequestId;
    use crate::pus::verification::{TcStateAccepted, VerificationToken};
    use crate::pus::{
        EcssTcInSharedStoreConverter, EcssTcInVecConverter, PusPacketHandlerResult,
        PusPacketHandlingError,
    };
    use delegate::delegate;
    use spacepackets::ecss::tc::{PusTcCreator, PusTcSecondaryHeader};
    use spacepackets::ecss::tm::PusTmReader;
    use spacepackets::ecss::PusPacket;
    use spacepackets::{SequenceFlags, SpHeader};

    use super::PusService17TestHandler;

    struct Pus17HandlerWithStoreTester {
        common: PusServiceHandlerWithSharedStoreCommon,
        handler: PusService17TestHandler<EcssTcInSharedStoreConverter>,
    }

    impl Pus17HandlerWithStoreTester {
        pub fn new() -> Self {
            let (common, srv_handler) = PusServiceHandlerWithSharedStoreCommon::new();
            let pus_17_handler = PusService17TestHandler::new(srv_handler);
            Self {
                common,
                handler: pus_17_handler,
            }
        }
    }

    impl PusTestHarness for Pus17HandlerWithStoreTester {
        delegate! {
            to self.common {
                fn send_tc(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted>;
                fn read_next_tm(&mut self) -> PusTmReader<'_>;
                fn check_no_tm_available(&self) -> bool;
                fn check_next_verification_tm(
                    &self,
                    subservice: u8,
                    expected_request_id: RequestId
                );
            }
        }
    }
    impl SimplePusPacketHandler for Pus17HandlerWithStoreTester {
        delegate! {
            to self.handler {
                fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError>;
            }
        }
    }

    struct Pus17HandlerWithVecTester {
        common: PusServiceHandlerWithVecCommon,
        handler: PusService17TestHandler<EcssTcInVecConverter>,
    }

    impl Pus17HandlerWithVecTester {
        pub fn new() -> Self {
            let (common, srv_handler) = PusServiceHandlerWithVecCommon::new();
            Self {
                common,
                handler: PusService17TestHandler::new(srv_handler),
            }
        }
    }

    impl PusTestHarness for Pus17HandlerWithVecTester {
        delegate! {
            to self.common {
                fn send_tc(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted>;
                fn read_next_tm(&mut self) -> PusTmReader<'_>;
                fn check_no_tm_available(&self) -> bool;
                fn check_next_verification_tm(
                    &self,
                    subservice: u8,
                    expected_request_id: RequestId,
                );
            }
        }
    }
    impl SimplePusPacketHandler for Pus17HandlerWithVecTester {
        delegate! {
            to self.handler {
                fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError>;
            }
        }
    }

    fn ping_test(test_harness: &mut (impl PusTestHarness + SimplePusPacketHandler)) {
        // Create a ping TC, verify acceptance.
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(17, 1);
        let ping_tc = PusTcCreator::new_no_app_data(&mut sp_header, sec_header, true);
        let token = test_harness.send_tc(&ping_tc);
        let request_id = token.req_id();
        let result = test_harness.handle_one_tc();
        assert!(result.is_ok());
        // We should see 4 replies in the TM queue now: Acceptance TM, Start TM, ping reply and
        // Completion TM

        // Acceptance TM
        test_harness.check_next_verification_tm(1, request_id);

        // Start TM
        test_harness.check_next_verification_tm(3, request_id);

        // Ping reply
        let tm = test_harness.read_next_tm();
        assert_eq!(tm.service(), 17);
        assert_eq!(tm.subservice(), 2);
        assert!(tm.user_data().is_empty());

        // TM completion
        test_harness.check_next_verification_tm(7, request_id);
    }

    #[test]
    fn test_basic_ping_processing_using_store() {
        let mut test_harness = Pus17HandlerWithStoreTester::new();
        ping_test(&mut test_harness);
    }

    #[test]
    fn test_basic_ping_processing_using_vec() {
        let mut test_harness = Pus17HandlerWithVecTester::new();
        ping_test(&mut test_harness);
    }

    #[test]
    fn test_empty_tc_queue() {
        let mut test_harness = Pus17HandlerWithStoreTester::new();
        let result = test_harness.handle_one_tc();
        assert!(result.is_ok());
        let result = result.unwrap();
        if let PusPacketHandlerResult::Empty = result {
        } else {
            panic!("unexpected result type {result:?}")
        }
    }

    #[test]
    fn test_sending_unsupported_service() {
        let mut test_harness = Pus17HandlerWithStoreTester::new();
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(3, 1);
        let ping_tc = PusTcCreator::new_no_app_data(&mut sp_header, sec_header, true);
        test_harness.send_tc(&ping_tc);
        let result = test_harness.handle_one_tc();
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let PusPacketHandlingError::WrongService(num) = error {
            assert_eq!(num, 3);
        } else {
            panic!("unexpected error type {error}")
        }
    }

    #[test]
    fn test_sending_custom_subservice() {
        let mut test_harness = Pus17HandlerWithStoreTester::new();
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(17, 200);
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
}
