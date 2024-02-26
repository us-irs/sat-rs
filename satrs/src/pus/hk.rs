pub use spacepackets::ecss::hk::*;

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub use std_mod::*;

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
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
pub mod alloc_mod {
    use spacepackets::ecss::tc::PusTcReader;

    use crate::pus::verification::VerificationReportingProvider;

    use super::*;

    /// This trait is an abstraction for the conversion of a PUS service 8 action telecommand into
    /// a [HkRequest].
    ///
    /// Having a dedicated trait for this allows maximum flexiblity and tailoring of the standard.
    /// The only requirement is that a valid [TargetId] and a [HkRequest] are returned by the
    /// core conversion function.
    ///
    /// The user should take care of performing the error handling as well. Some of the following
    /// aspects might be relevant:
    ///
    /// - Checking the validity of the APID, service ID, subservice ID.
    /// - Checking the validity of the user data.
    ///
    /// A [VerificationReportingProvider] is passed to the user to also allow handling
    /// of the verification process as part of the PUS standard requirements.
    pub trait PusHkToRequestConverter {
        type Error;
        fn convert(
            &mut self,
            token: VerificationToken<TcStateAccepted>,
            tc: &PusTcReader,
            time_stamp: &[u8],
            verif_reporter: &impl VerificationReportingProvider,
        ) -> Result<(TargetId, HkRequest), Self::Error>;
    }
}

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub mod std_mod {
    use crate::pus::{
        get_current_cds_short_timestamp, verification::VerificationReportingProvider,
        EcssTcInMemConverter, EcssTcReceiverCore, EcssTmSenderCore, GenericRoutingError,
        PusPacketHandlerResult, PusPacketHandlingError, PusRoutingErrorHandler, PusServiceHelper,
    };

    use super::*;

    /// This is a generic high-level handler for the PUS service 3 housekeeping service.
    ///
    /// It performs the following handling steps:
    ///
    /// 1. Retrieve the next TC packet from the [PusServiceHelper]. The [EcssTcInMemConverter]
    ///    allows to configure the used telecommand memory backend.
    /// 2. Convert the TC to a targeted action request using the provided
    ///    [PusHkToRequestConverter]. The generic error type is constrained to the
    ///    [PusPacketHandlerResult] for the concrete implementation which offers a packet handler.
    /// 3. Route the action request using the provided [PusHkRequestRouter]. The generic error
    ///    type is constrained to the [GenericRoutingError] for the concrete implementation.
    /// 4. Handle all routing errors using the provided [PusRoutingErrorHandler]. The generic error
    ///    type is constrained to the [GenericRoutingError] for the concrete implementation.
    pub struct PusService3HkHandler<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
        RequestConverter: PusHkToRequestConverter,
        RequestRouter: PusHkRequestRouter<Error = RoutingError>,
        RoutingErrorHandler: PusRoutingErrorHandler<Error = RoutingError>,
        RoutingError = GenericRoutingError,
    > {
        service_helper:
            PusServiceHelper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
        pub request_converter: RequestConverter,
        pub request_router: RequestRouter,
        pub routing_error_handler: RoutingErrorHandler,
    }

    impl<
            TcReceiver: EcssTcReceiverCore,
            TmSender: EcssTmSenderCore,
            TcInMemConverter: EcssTcInMemConverter,
            VerificationReporter: VerificationReportingProvider,
            RequestConverter: PusHkToRequestConverter<Error = PusPacketHandlingError>,
            RequestRouter: PusHkRequestRouter<Error = RoutingError>,
            RoutingErrorHandler: PusRoutingErrorHandler<Error = RoutingError>,
            RoutingError: Clone,
        >
        PusService3HkHandler<
            TcReceiver,
            TmSender,
            TcInMemConverter,
            VerificationReporter,
            RequestConverter,
            RequestRouter,
            RoutingErrorHandler,
            RoutingError,
        >
    where
        PusPacketHandlingError: From<RoutingError>,
    {
        pub fn new(
            service_helper: PusServiceHelper<
                TcReceiver,
                TmSender,
                TcInMemConverter,
                VerificationReporter,
            >,
            request_converter: RequestConverter,
            request_router: RequestRouter,
            routing_error_handler: RoutingErrorHandler,
        ) -> Self {
            Self {
                service_helper,
                request_converter,
                request_router,
                routing_error_handler,
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
            let mut partial_error = None;
            let time_stamp = get_current_cds_short_timestamp(&mut partial_error);
            let (target_id, hk_request) = self.request_converter.convert(
                ecss_tc_and_token.token,
                &tc,
                &time_stamp,
                &self.service_helper.common.verification_handler,
            )?;
            if let Err(e) =
                self.request_router
                    .route(target_id, hk_request, ecss_tc_and_token.token)
            {
                self.routing_error_handler.handle_error(
                    target_id,
                    ecss_tc_and_token.token,
                    &tc,
                    e.clone(),
                    &time_stamp,
                    &self.service_helper.common.verification_handler,
                );
                return Err(e.into());
            }
            Ok(PusPacketHandlerResult::RequestHandled)
        }
    }
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

    use crate::pus::{MpscTcReceiver, TmAsVecSenderWithMpsc};
    use crate::{
        hk::HkRequest,
        pus::{
            tests::{
                PusServiceHandlerWithVecCommon, PusTestHarness, SimplePusPacketHandler,
                TestConverter, TestRouter, TestRoutingErrorHandler, APP_DATA_TOO_SHORT, TEST_APID,
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

    use super::{PusHkRequestRouter, PusHkToRequestConverter, PusService3HkHandler};

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

    impl PusHkToRequestConverter for TestConverter<3> {
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
        handler: PusService3HkHandler<
            MpscTcReceiver,
            TmAsVecSenderWithMpsc,
            EcssTcInVecConverter,
            TestVerificationReporter,
            TestConverter<3>,
            TestRouter<HkRequest>,
            TestRoutingErrorHandler,
        >,
    }

    impl Pus3HandlerWithVecTester {
        pub fn new() -> Self {
            let (common, srv_handler) =
                PusServiceHandlerWithVecCommon::new_with_test_verif_sender();
            Self {
                common,
                handler: PusService3HkHandler::new(
                    srv_handler,
                    TestConverter::default(),
                    TestRouter::default(),
                    TestRoutingErrorHandler::default(),
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
            to self.handler.routing_error_handler {
                pub fn retrieve_next_error(&mut self) -> (TargetId, GenericRoutingError);
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

        let (target_id, found_error) = hk_handler.retrieve_next_error();
        assert_eq!(target_id, TEST_APID.into());
        check_error(found_error);
    }
}
