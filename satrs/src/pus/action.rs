use crate::{
    action::{ActionId, ActionRequest},
    params::Params,
    request::RequestId,
    TargetId,
};

use super::verification::{TcStateAccepted, VerificationToken};

use satrs_shared::res_code::ResultU16;
use spacepackets::ecss::EcssEnumU16;

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub use std_mod::*;

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub use alloc_mod::*;

#[derive(Clone, Debug)]
pub struct ActionRequestWithId {
    pub request_id: RequestId,
    pub request: ActionRequest,
}

#[derive(Clone, Debug)]
pub struct ActionReplyPusWithIds {
    pub request_id: RequestId,
    pub action_id: ActionId,
    pub reply: ActionReplyPus,
}

/// A reply to an action request, but tailored to the PUS standard verification process.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum ActionReplyPus {
    Completed,
    StepSuccess {
        step: EcssEnumU16,
    },
    CompletionFailed {
        error_code: ResultU16,
        params: Params,
    },
    StepFailed {
        error_code: ResultU16,
        step: EcssEnumU16,
        params: Params,
    },
}

/// This trait is an abstraction for the routing of PUS service 8 action requests to a dedicated
/// recipient using the generic [TargetId].
pub trait PusActionRequestRouter {
    type Error;
    fn route(
        &self,
        target_id: TargetId,
        hk_request: ActionRequest,
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
    /// an [ActionRequest].
    ///
    /// Having a dedicated trait for this allows maximum flexiblity and tailoring of the standard.
    /// The only requirement is that a valid [TargetId] and an [ActionRequest] are returned by the
    /// core conversion function.
    ///
    /// The user should take care of performing the error handling as well. Some of the following
    /// aspects might be relevant:
    ///
    /// - Checking the validity of the APID, service ID, subservice ID.
    /// - Checking the validity of the user data.
    ///
    /// A [VerificationReportingProvider] instance is passed to the user to also allow handling
    /// of the verification process as part of the PUS standard requirements.
    pub trait PusActionToRequestConverter {
        type Error;
        fn convert(
            &mut self,
            token: VerificationToken<TcStateAccepted>,
            tc: &PusTcReader,
            time_stamp: &[u8],
            verif_reporter: &impl VerificationReportingProvider,
        ) -> Result<(TargetId, ActionRequest), Self::Error>;
    }
}

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub mod std_mod {
    use crate::{
        params::WritableToBeBytes,
        pus::{
            get_current_cds_short_timestamp,
            verification::{
                self, FailParams, FailParamsWithStep, TcStateStarted, VerificationReportingProvider,
            },
            EcssTcInMemConverter, EcssTcReceiverCore, EcssTmSenderCore, EcssTmtcError,
            GenericRoutingError, PusPacketHandlerResult, PusPacketHandlingError,
            PusRoutingErrorHandler, PusServiceHelper,
        },
        request::RequestId,
    };
    use core::time::Duration;
    use hashbrown::HashMap;
    use spacepackets::time::UnixTimestamp;
    use std::time::SystemTimeError;

    use super::*;

    /// This is a high-level handler for the PUS service 8 action service.
    ///
    /// It performs the following handling steps:
    ///
    /// 1. Retrieve the next TC packet from the [PusServiceHelper]. The [EcssTcInMemConverter]
    ///    allows to configure the used telecommand memory backend.
    /// 2. Convert the TC to a targeted action request using the provided
    ///    [PusActionToRequestConverter]. The generic error type is constrained to the
    ///    [PusPacketHandlingError] for the concrete implementation which offers a packet handler.
    /// 3. Route the action request using the provided [PusActionRequestRouter].
    /// 4. Handle all routing errors using the provided [PusRoutingErrorHandler].
    pub struct PusService8ActionHandler<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
        RequestConverter: PusActionToRequestConverter,
        RequestRouter: PusActionRequestRouter<Error = RoutingError>,
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
            RequestConverter: PusActionToRequestConverter<Error = PusPacketHandlingError>,
            RequestRouter: PusActionRequestRouter<Error = RoutingError>,
            RoutingErrorHandler: PusRoutingErrorHandler<Error = RoutingError>,
            RoutingError: Clone,
        >
        PusService8ActionHandler<
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

        /// Core function to poll the next TC packet and try to handle it.
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
            let (target_id, action_request) = self.request_converter.convert(
                ecss_tc_and_token.token,
                &tc,
                &time_stamp,
                &self.service_helper.common.verification_handler,
            )?;
            if let Err(e) =
                self.request_router
                    .route(target_id, action_request, ecss_tc_and_token.token)
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ActiveActionRequest {
        action_id: ActionId,
        token: VerificationToken<TcStateStarted>,
        start_time: UnixTimestamp,
        timeout: Duration,
    }

    pub trait ActionReplyHandlerHook {
        fn handle_unexpected_reply(&mut self, reply: &ActionReplyPusWithIds);
        fn timeout_callback(&self, active_request: &ActiveActionRequest);
        fn timeout_error_code(&self) -> ResultU16;
    }

    pub struct PusService8ReplyHandler<
        VerificationReporter: VerificationReportingProvider,
        UserHook: ActionReplyHandlerHook,
    > {
        active_requests: HashMap<RequestId, ActiveActionRequest>,
        verification_reporter: VerificationReporter,
        fail_data_buf: alloc::vec::Vec<u8>,
        current_time: UnixTimestamp,
        user_hook: UserHook,
    }

    impl<VerificationReporter: VerificationReportingProvider, UserHook: ActionReplyHandlerHook>
        PusService8ReplyHandler<VerificationReporter, UserHook>
    {
        pub fn new(
            verification_reporter: VerificationReporter,
            fail_data_buf_size: usize,
            user_hook: UserHook,
        ) -> Self {
            Self {
                active_requests: HashMap::new(),
                verification_reporter,
                fail_data_buf: alloc::vec![0; fail_data_buf_size],
                current_time: UnixTimestamp::from_now().unwrap(),
                user_hook,
            }
        }
        pub fn add_routed_request(
            &mut self,
            request_id: verification::RequestId,
            action_id: ActionId,
            token: VerificationToken<TcStateStarted>,
            timeout: Duration,
        ) {
            self.active_requests.insert(
                request_id.into(),
                ActiveActionRequest {
                    action_id,
                    token,
                    start_time: self.current_time,
                    timeout,
                },
            );
        }

        #[cfg(feature = "std")]
        #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
        pub fn update_time_from_now(&mut self) -> Result<(), SystemTimeError> {
            self.current_time = UnixTimestamp::from_now()?;
            Ok(())
        }

        pub fn check_for_timeouts(&mut self, time_stamp: &[u8]) -> Result<(), EcssTmtcError> {
            for active_req in self.active_requests.values() {
                let diff = self.current_time - active_req.start_time;
                if diff.duration_absolute > active_req.timeout {
                    self.handle_timeout(active_req, time_stamp);
                }
            }
            Ok(())
        }

        pub fn handle_timeout(&self, active_request: &ActiveActionRequest, time_stamp: &[u8]) {
            self.verification_reporter
                .completion_failure(
                    active_request.token,
                    FailParams::new(time_stamp, &self.user_hook.timeout_error_code(), &[]),
                )
                .unwrap();
            self.user_hook.timeout_callback(active_request);
        }

        pub fn handle_action_reply(
            &mut self,
            action_reply_with_ids: ActionReplyPusWithIds,
            time_stamp: &[u8],
        ) -> Result<(), EcssTmtcError> {
            let active_req = self.active_requests.get(&action_reply_with_ids.request_id);
            if active_req.is_none() {
                self.user_hook
                    .handle_unexpected_reply(&action_reply_with_ids);
            }
            let active_req = active_req.unwrap();
            match action_reply_with_ids.reply {
                ActionReplyPus::CompletionFailed { error_code, params } => {
                    params.write_to_be_bytes(&mut self.fail_data_buf)?;
                    self.verification_reporter
                        .completion_failure(
                            active_req.token,
                            FailParams::new(time_stamp, &error_code, &self.fail_data_buf),
                        )
                        .map_err(|e| e.0)?;
                    self.active_requests
                        .remove(&action_reply_with_ids.request_id);
                }
                ActionReplyPus::StepFailed {
                    error_code,
                    step,
                    params,
                } => {
                    params.write_to_be_bytes(&mut self.fail_data_buf)?;
                    self.verification_reporter
                        .step_failure(
                            active_req.token,
                            FailParamsWithStep::new(
                                time_stamp,
                                &step,
                                &error_code,
                                &self.fail_data_buf,
                            ),
                        )
                        .map_err(|e| e.0)?;
                    self.active_requests
                        .remove(&action_reply_with_ids.request_id);
                }
                ActionReplyPus::Completed => {
                    self.verification_reporter
                        .completion_success(active_req.token, time_stamp)
                        .map_err(|e| e.0)?;
                    self.active_requests
                        .remove(&action_reply_with_ids.request_id);
                }
                ActionReplyPus::StepSuccess { step } => {
                    self.verification_reporter
                        .step_success(&active_req.token, time_stamp, step)?;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;

    use alloc::collections::VecDeque;
    use delegate::delegate;

    use spacepackets::{
        ecss::{
            tc::{PusTcCreator, PusTcReader, PusTcSecondaryHeader},
            tm::PusTmReader,
            PusPacket,
        },
        CcsdsPacket, SequenceFlags, SpHeader,
    };

    use crate::{
        action::ActionRequestVariant,
        pus::{
            tests::{
                PusServiceHandlerWithVecCommon, PusTestHarness, SimplePusPacketHandler,
                TestConverter, TestRouter, TestRoutingErrorHandler, APP_DATA_TOO_SHORT, TEST_APID,
            },
            verification::{
                tests::{TestVerificationReporter, SharedVerificationMap}, FailParams, RequestId,
                VerificationReportingProvider,
            },
            EcssTcInVecConverter, GenericRoutingError, MpscTcReceiver, PusPacketHandlerResult,
            PusPacketHandlingError, TmAsVecSenderWithMpsc,
        },
    };

    use super::*;

    impl PusActionRequestRouter for TestRouter<ActionRequest> {
        type Error = GenericRoutingError;

        fn route(
            &self,
            target_id: TargetId,
            action_request: ActionRequest,
            _token: VerificationToken<TcStateAccepted>,
        ) -> Result<(), Self::Error> {
            self.routing_requests
                .borrow_mut()
                .push_back((target_id, action_request));
            self.check_for_injected_error()
        }
    }

    impl PusActionToRequestConverter for TestConverter<8> {
        type Error = PusPacketHandlingError;
        fn convert(
            &mut self,
            token: VerificationToken<TcStateAccepted>,
            tc: &PusTcReader,
            time_stamp: &[u8],
            verif_reporter: &impl VerificationReportingProvider,
        ) -> Result<(TargetId, ActionRequest), Self::Error> {
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
            if tc.subservice() == 1 {
                verif_reporter
                    .start_success(token, time_stamp)
                    .expect("start success failure");
                return Ok((
                    target_id.into(),
                    ActionRequest {
                        action_id: u32::from_be_bytes(tc.user_data()[0..4].try_into().unwrap()),
                        variant: ActionRequestVariant::VecData(tc.user_data()[4..].to_vec()),
                    },
                ));
            }
            Err(PusPacketHandlingError::InvalidAppData(
                "unexpected app data".into(),
            ))
        }
    }

    struct Pus8HandlerWithVecTester {
        common: PusServiceHandlerWithVecCommon<TestVerificationReporter>,
        handler: PusService8ActionHandler<
            MpscTcReceiver,
            TmAsVecSenderWithMpsc,
            EcssTcInVecConverter,
            TestVerificationReporter,
            TestConverter<8>,
            TestRouter<ActionRequest>,
            TestRoutingErrorHandler,
        >,
    }

    impl Pus8HandlerWithVecTester {
        pub fn new() -> Self {
            let (common, srv_handler) =
                PusServiceHandlerWithVecCommon::new_with_test_verif_sender();
            Self {
                common,
                handler: PusService8ActionHandler::new(
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
                pub fn retrieve_next_request(&mut self) -> (TargetId, ActionRequest);
            }
        }
        delegate! {
            to self.handler.routing_error_handler {
                pub fn retrieve_next_error(&mut self) -> (TargetId, GenericRoutingError);
            }
        }
    }

    impl PusTestHarness for Pus8HandlerWithVecTester {
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
    impl SimplePusPacketHandler for Pus8HandlerWithVecTester {
        delegate! {
            to self.handler {
                fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError>;
            }
        }
    }

    const TIMEOUT_ERROR_CODE: ResultU16 = ResultU16::new(1, 2);

    #[derive(Default)]
    pub struct TestReplyHandlerHook {
        pub unexpected_replies: VecDeque<ActionReplyPusWithIds>,
        pub timeouts: RefCell<VecDeque<ActiveActionRequest>>,
    }

    impl ActionReplyHandlerHook for TestReplyHandlerHook {
        fn handle_unexpected_reply(&mut self, reply: &ActionReplyPusWithIds) {
            self.unexpected_replies.push_back(reply.clone());
        }

        fn timeout_callback(&self, active_request: &ActiveActionRequest) {
            self.timeouts.borrow_mut().push_back(active_request.clone());
        }

        fn timeout_error_code(&self) -> ResultU16 {
            TIMEOUT_ERROR_CODE
        }
    }

    #[test]
    fn basic_test() {
        let mut action_handler = Pus8HandlerWithVecTester::new();
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(8, 1);
        let action_id: u32 = 1;
        let action_id_raw = action_id.to_be_bytes();
        let tc = PusTcCreator::new(&mut sp_header, sec_header, action_id_raw.as_ref(), true);
        action_handler.send_tc(&tc);
        let result = action_handler.handle_one_tc();
        assert!(result.is_ok());
        action_handler.check_next_conversion(&tc);
        let (target_id, action_req) = action_handler.retrieve_next_request();
        assert_eq!(target_id, TEST_APID.into());
        assert_eq!(action_req.action_id, 1);
        if let ActionRequestVariant::VecData(data) = action_req.variant {
            assert_eq!(data, &[]);
        }
    }

    #[test]
    fn test_routing_error() {
        let mut action_handler = Pus8HandlerWithVecTester::new();
        let mut sp_header = SpHeader::tc(TEST_APID, SequenceFlags::Unsegmented, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(8, 1);
        let action_id: u32 = 1;
        let action_id_raw = action_id.to_be_bytes();
        let tc = PusTcCreator::new(&mut sp_header, sec_header, action_id_raw.as_ref(), true);
        let error = GenericRoutingError::UnknownTargetId(25);
        action_handler
            .handler
            .request_router
            .inject_routing_error(error);
        action_handler.send_tc(&tc);
        let result = action_handler.handle_one_tc();
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

        action_handler.check_next_conversion(&tc);
        let (target_id, action_req) = action_handler.retrieve_next_request();
        assert_eq!(target_id, TEST_APID.into());
        assert_eq!(action_req.action_id, 1);
        if let ActionRequestVariant::VecData(data) = action_req.variant {
            assert_eq!(data, &[]);
        }

        let (target_id, found_error) = action_handler.retrieve_next_error();
        assert_eq!(target_id, TEST_APID.into());
        check_error(found_error);
    }

    #[test]
    fn test_reply_handler() {
        let reply_handler_hook = TestReplyHandlerHook::default();
        let test_verif_reporter = TestVerificationReporter::default();
        //let reply_handler = PusService8ReplyHandler::new(verification_reporter, fail_data_buf_size, user_hook)

    }
}
