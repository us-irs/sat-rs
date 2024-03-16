pub use spacepackets::ecss::hk::*;

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub use std_mod::*;

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
#[allow(unused_imports)]
pub use alloc_mod::*;

use crate::{hk::HkRequest, TargetId};

use super::verification::{TcStateAccepted, VerificationToken};

/// This trait is an abstraction for the routing of PUS service 3 housekeeping requests to a
/// dedicated recipient using the generic [TargetId].
pub trait PusHkRequestRouter {
    type Error;
    fn route(
        &self,
        target_id: TargetId,
        hk_request: HkRequest,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<(), Self::Error>;
}

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod alloc_mod {}

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub mod std_mod {
    use crate::pus::{GenericRoutingError, PusTargetedRequestHandler};

    use super::*;

    pub type PusService3HkRequestHandler<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        RequestConverter,
        RequestRouter,
        RoutingError = GenericRoutingError,
    > = PusTargetedRequestHandler<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        RequestConverter,
        RequestRouter,
        HkRequest,
        RoutingError,
    >;
}

#[cfg(test)]
mod tests {
    use delegate::delegate;
    use spacepackets::ecss::hk::Subservice;

    use spacepackets::{
        ecss::{
            tc::{PusTcCreator, PusTcReader, PusTcSecondaryHeader},
            tm::PusTmReader,
            PusPacket,
        },
        CcsdsPacket, SequenceFlags, SpHeader,
    };

    use crate::pus::{MpscTcReceiver, PusTcToRequestConverter, TmAsVecSenderWithMpsc};
    use crate::{
        hk::HkRequest,
        pus::{
            tests::{
                PusServiceHandlerWithVecCommon, PusTestHarness, SimplePusPacketHandler,
                TestConverter, TestRouter, APP_DATA_TOO_SHORT, TEST_APID,
            },
            verification::{
                tests::TestVerificationReporter, FailParams, RequestId, TcStateAccepted,
                VerificationReportingProvider, VerificationToken,
            },
            EcssTcInVecConverter, GenericRoutingError, PusPacketHandlerResult,
            PusPacketHandlingError,
        },
        TargetId,
    };

    use super::{PusHkRequestRouter, PusService3HkRequestHandler};

    impl PusHkRequestRouter for TestRouter<HkRequest> {
        type Error = GenericRoutingError;

        fn route(
            &self,
            target_id: TargetId,
            hk_request: HkRequest,
            _token: VerificationToken<TcStateAccepted>,
        ) -> Result<(), Self::Error> {
            self.routing_requests
                .borrow_mut()
                .push_back((target_id, hk_request));
            self.check_for_injected_error()
        }
    }

    impl PusTcToRequestConverter<HkRequest> for TestConverter<3> {
        type Error = PusPacketHandlingError;
        fn convert(
            &mut self,
            token: VerificationToken<TcStateAccepted>,
            tc: &PusTcReader,
            time_stamp: &[u8],
            verif_reporter: &impl VerificationReportingProvider,
        ) -> Result<(TargetId, HkRequest), Self::Error> {
            self.conversion_request.push_back(tc.raw_data().to_vec());
            self.check_service(tc)?;
            let target_id = tc.apid();
            if tc.user_data().len() < 4 {
                verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(
                            time_stamp,
                            &APP_DATA_TOO_SHORT,
                            (tc.user_data().len() as u32).to_be_bytes().as_ref(),
                        ),
                    )
                    .expect("start success failure");
                return Err(PusPacketHandlingError::NotEnoughAppData {
                    expected: 4,
                    found: tc.user_data().len(),
                });
            }
            if tc.subservice() == Subservice::TcGenerateOneShotHk as u8 {
                verif_reporter
                    .start_success(token, time_stamp)
                    .expect("start success failure");
                return Ok((
                    target_id.into(),
                    HkRequest::OneShot(u32::from_be_bytes(
                        tc.user_data()[0..4].try_into().unwrap(),
                    )),
                ));
            }
            Err(PusPacketHandlingError::InvalidAppData(
                "unexpected app data".into(),
            ))
        }
    }

    struct Pus3HandlerWithVecTester {
        common: PusServiceHandlerWithVecCommon<TestVerificationReporter>,
        handler: PusService3HkRequestHandler<
            MpscTcReceiver,
            TmAsVecSenderWithMpsc,
            EcssTcInVecConverter,
            TestVerificationReporter,
            TestConverter<3>,
            TestRouter<HkRequest>,
        >,
    }

    impl Pus3HandlerWithVecTester {
        pub fn new() -> Self {
            let (common, srv_handler) =
                PusServiceHandlerWithVecCommon::new_with_test_verif_sender();
            Self {
                common,
                handler: PusService3HkRequestHandler::new(
                    srv_handler,
                    TestConverter::default(),
                    TestRouter::default(),
                ),
            }
        }

        delegate! {
            to self.handler.request_converter {
                pub fn check_next_conversion(&mut self, tc: &PusTcCreator);
            }
        }
        delegate! {
            to self.handler.request_router {
                pub fn retrieve_next_request(&mut self) -> (TargetId, HkRequest);
            }
        }
        delegate! {
            to self.handler.request_router  {
                pub fn retrieve_next_routing_error(&mut self) -> (TargetId, GenericRoutingError);
            }
        }
    }

    impl PusTestHarness for Pus3HandlerWithVecTester {
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
    impl SimplePusPacketHandler for Pus3HandlerWithVecTester {
        delegate! {
            to self.handler {
                fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError>;
            }
        }
    }

    #[test]
    fn basic_test() {
        let mut hk_handler = Pus3HandlerWithVecTester::new();
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(3, Subservice::TcGenerateOneShotHk as u8);
        let unique_id: u32 = 1;
        let unique_id_raw = unique_id.to_be_bytes();
        let tc = PusTcCreator::new(&mut sp_header, sec_header, unique_id_raw.as_ref(), true);
        hk_handler.send_tc(&tc);
        let result = hk_handler.handle_one_tc();
        assert!(result.is_ok());
        hk_handler.check_next_conversion(&tc);
        let (target_id, hk_request) = hk_handler.retrieve_next_request();
        assert_eq!(target_id, TEST_APID.into());
        if let HkRequest::OneShot(id) = hk_request {
            assert_eq!(id, unique_id);
        } else {
            panic!("unexpected request");
        }
    }

    #[test]
    fn test_routing_error() {
        let mut hk_handler = Pus3HandlerWithVecTester::new();
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(3, Subservice::TcGenerateOneShotHk as u8);
        let unique_id: u32 = 1;
        let unique_id_raw = unique_id.to_be_bytes();
        let tc = PusTcCreator::new(&mut sp_header, sec_header, unique_id_raw.as_ref(), true);
        let error = GenericRoutingError::UnknownTargetId(25);
        hk_handler
            .handler
            .request_router
            .inject_routing_error(error);
        hk_handler.send_tc(&tc);
        let result = hk_handler.handle_one_tc();
        assert!(result.is_err());
        let check_error = |routing_error: GenericRoutingError| {
            if let GenericRoutingError::UnknownTargetId(id) = routing_error {
                assert_eq!(id, 25);
            } else {
                panic!("unexpected error type");
            }
        };
        if let PusPacketHandlingError::RequestRoutingError(routing_error) = result.unwrap_err() {
            check_error(routing_error);
        } else {
            panic!("unexpected error type");
        }

        hk_handler.check_next_conversion(&tc);
        let (target_id, hk_req) = hk_handler.retrieve_next_request();
        assert_eq!(target_id, TEST_APID.into());
        if let HkRequest::OneShot(unique_id) = hk_req {
            assert_eq!(unique_id, 1);
        }

        let (target_id, found_error) = hk_handler.retrieve_next_routing_error();
        assert_eq!(target_id, TEST_APID.into());
        check_error(found_error);
    }
}
