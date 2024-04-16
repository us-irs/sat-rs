use crate::pus::{
    PartialPusHandlingError, PusPacketHandlerResult, PusPacketHandlingError, PusTmVariant,
};
use crate::tmtc::{PacketAsVec, PacketSenderWithSharedPool};
use spacepackets::ecss::tm::{PusTmCreator, PusTmSecondaryHeader};
use spacepackets::ecss::PusPacket;
use spacepackets::SpHeader;
use std::sync::mpsc;

use super::verification::{VerificationReporter, VerificationReportingProvider};
use super::{
    EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter, EcssTcReceiver,
    EcssTmSender, GenericConversionError, MpscTcReceiver, PusServiceHelper,
};

/// This is a helper class for [std] environments to handle generic PUS 17 (test service) packets.
/// This handler only processes ping requests and generates a ping reply for them accordingly.
pub struct PusService17TestHandler<
    TcReceiver: EcssTcReceiver,
    TmSender: EcssTmSender,
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    pub service_helper:
        PusServiceHelper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
}

impl<
        TcReceiver: EcssTcReceiver,
        TmSender: EcssTmSender,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > PusService17TestHandler<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>
{
    pub fn new(
        service_helper: PusServiceHelper<
            TcReceiver,
            TmSender,
            TcInMemConverter,
            VerificationReporter,
        >,
    ) -> Self {
        Self { service_helper }
    }

    pub fn poll_and_handle_next_tc(
        &mut self,
        time_stamp: &[u8],
    ) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        let possible_packet = self.service_helper.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(PusPacketHandlerResult::Empty);
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        self.service_helper
            .tc_in_mem_converter_mut()
            .cache(&ecss_tc_and_token.tc_in_memory)?;
        let tc = self.service_helper.tc_in_mem_converter().convert()?;
        if tc.service() != 17 {
            return Err(GenericConversionError::WrongService(tc.service()).into());
        }
        if tc.subservice() == 1 {
            let mut partial_error = None;
            let result = self
                .service_helper
                .verif_reporter()
                .start_success(
                    &self.service_helper.common.tm_sender,
                    ecss_tc_and_token.token,
                    time_stamp,
                )
                .map_err(|_| PartialPusHandlingError::Verification);
            let start_token = if let Ok(result) = result {
                Some(result)
            } else {
                partial_error = Some(result.unwrap_err());
                None
            };
            // Sequence count will be handled centrally in TM funnel.
            // It is assumed that the verification reporter was built with a valid APID, so we use
            // the unchecked API here.
            let reply_header =
                SpHeader::new_for_unseg_tm(self.service_helper.verif_reporter().apid(), 0, 0);
            let tc_header = PusTmSecondaryHeader::new_simple(17, 2, time_stamp);
            let ping_reply = PusTmCreator::new(reply_header, tc_header, &[], true);
            let result = self
                .service_helper
                .common
                .tm_sender
                .send_tm(self.service_helper.id(), PusTmVariant::Direct(ping_reply))
                .map_err(PartialPusHandlingError::TmSend);
            if let Err(err) = result {
                partial_error = Some(err);
            }

            if let Some(start_token) = start_token {
                if self
                    .service_helper
                    .verif_reporter()
                    .completion_success(
                        &self.service_helper.common.tm_sender,
                        start_token,
                        time_stamp,
                    )
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

/// Helper type definition for a PUS 17 handler with a dynamic TMTC memory backend and regular
/// mpsc queues.
pub type PusService17TestHandlerDynWithMpsc = PusService17TestHandler<
    MpscTcReceiver,
    mpsc::Sender<PacketAsVec>,
    EcssTcInVecConverter,
    VerificationReporter,
>;
/// Helper type definition for a PUS 17 handler with a dynamic TMTC memory backend and bounded MPSC
/// queues.
pub type PusService17TestHandlerDynWithBoundedMpsc = PusService17TestHandler<
    MpscTcReceiver,
    mpsc::SyncSender<PacketAsVec>,
    EcssTcInVecConverter,
    VerificationReporter,
>;
/// Helper type definition for a PUS 17 handler with a shared store TMTC memory backend and bounded
/// mpsc queues.
pub type PusService17TestHandlerStaticWithBoundedMpsc = PusService17TestHandler<
    MpscTcReceiver,
    PacketSenderWithSharedPool,
    EcssTcInSharedStoreConverter,
    VerificationReporter,
>;

#[cfg(test)]
mod tests {
    use crate::pus::test_util::{PusTestHarness, SimplePusPacketHandler, TEST_APID};
    use crate::pus::tests::{
        PusServiceHandlerWithSharedStoreCommon, PusServiceHandlerWithVecCommon,
    };
    use crate::pus::verification::{
        RequestId, VerificationReporter, VerificationReportingProvider,
    };
    use crate::pus::verification::{TcStateAccepted, VerificationToken};
    use crate::pus::{
        EcssTcInSharedStoreConverter, EcssTcInVecConverter, GenericConversionError, MpscTcReceiver,
        MpscTmAsVecSender, PusPacketHandlerResult, PusPacketHandlingError,
    };
    use crate::tmtc::PacketSenderWithSharedPool;
    use crate::ComponentId;
    use delegate::delegate;
    use spacepackets::ecss::tc::{PusTcCreator, PusTcSecondaryHeader};
    use spacepackets::ecss::tm::PusTmReader;
    use spacepackets::ecss::PusPacket;
    use spacepackets::time::{cds, TimeWriter};
    use spacepackets::SpHeader;

    use super::PusService17TestHandler;

    struct Pus17HandlerWithStoreTester {
        common: PusServiceHandlerWithSharedStoreCommon,
        handler: PusService17TestHandler<
            MpscTcReceiver,
            PacketSenderWithSharedPool,
            EcssTcInSharedStoreConverter,
            VerificationReporter,
        >,
    }

    impl Pus17HandlerWithStoreTester {
        pub fn new(id: ComponentId) -> Self {
            let (common, srv_handler) = PusServiceHandlerWithSharedStoreCommon::new(id);
            let pus_17_handler = PusService17TestHandler::new(srv_handler);
            Self {
                common,
                handler: pus_17_handler,
            }
        }
    }

    impl PusTestHarness for Pus17HandlerWithStoreTester {
        fn init_verification(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted> {
            let init_token = self.handler.service_helper.verif_reporter_mut().add_tc(tc);
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
                fn check_next_verification_tm(
                    &self,
                    subservice: u8,
                    expected_request_id: RequestId
                );
            }
        }
    }
    impl SimplePusPacketHandler for Pus17HandlerWithStoreTester {
        fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
            let time_stamp = cds::CdsTime::new_with_u16_days(0, 0).to_vec().unwrap();
            self.handler.poll_and_handle_next_tc(&time_stamp)
        }
    }

    struct Pus17HandlerWithVecTester {
        common: PusServiceHandlerWithVecCommon,
        handler: PusService17TestHandler<
            MpscTcReceiver,
            MpscTmAsVecSender,
            EcssTcInVecConverter,
            VerificationReporter,
        >,
    }

    impl Pus17HandlerWithVecTester {
        pub fn new(id: ComponentId) -> Self {
            let (common, srv_handler) =
                PusServiceHandlerWithVecCommon::new_with_standard_verif_reporter(id);
            Self {
                common,
                handler: PusService17TestHandler::new(srv_handler),
            }
        }
    }

    impl PusTestHarness for Pus17HandlerWithVecTester {
        fn init_verification(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted> {
            let init_token = self.handler.service_helper.verif_reporter_mut().add_tc(tc);
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
                fn check_next_verification_tm(
                    &self,
                    subservice: u8,
                    expected_request_id: RequestId,
                );
            }
        }
    }
    impl SimplePusPacketHandler for Pus17HandlerWithVecTester {
        fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
            let time_stamp = cds::CdsTime::new_with_u16_days(0, 0).to_vec().unwrap();
            self.handler.poll_and_handle_next_tc(&time_stamp)
        }
    }

    fn ping_test(test_harness: &mut (impl PusTestHarness + SimplePusPacketHandler)) {
        // Create a ping TC, verify acceptance.
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header = PusTcSecondaryHeader::new_simple(17, 1);
        let ping_tc = PusTcCreator::new_no_app_data(sp_header, sec_header, true);
        let token = test_harness.init_verification(&ping_tc);
        test_harness.send_tc(&token, &ping_tc);
        let request_id = token.request_id();
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
        let mut test_harness = Pus17HandlerWithStoreTester::new(0);
        ping_test(&mut test_harness);
    }

    #[test]
    fn test_basic_ping_processing_using_vec() {
        let mut test_harness = Pus17HandlerWithVecTester::new(0);
        ping_test(&mut test_harness);
    }

    #[test]
    fn test_empty_tc_queue() {
        let mut test_harness = Pus17HandlerWithStoreTester::new(0);
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
        let mut test_harness = Pus17HandlerWithStoreTester::new(0);
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header = PusTcSecondaryHeader::new_simple(3, 1);
        let ping_tc = PusTcCreator::new_no_app_data(sp_header, sec_header, true);
        let token = test_harness.init_verification(&ping_tc);
        test_harness.send_tc(&token, &ping_tc);
        let result = test_harness.handle_one_tc();
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let PusPacketHandlingError::RequestConversion(GenericConversionError::WrongService(
            num,
        )) = error
        {
            assert_eq!(num, 3);
        } else {
            panic!("unexpected error type {error}")
        }
    }

    #[test]
    fn test_sending_custom_subservice() {
        let mut test_harness = Pus17HandlerWithStoreTester::new(0);
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header = PusTcSecondaryHeader::new_simple(17, 200);
        let ping_tc = PusTcCreator::new_no_app_data(sp_header, sec_header, true);
        let token = test_harness.init_verification(&ping_tc);
        test_harness.send_tc(&token, &ping_tc);
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
