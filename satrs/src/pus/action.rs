use core::time::Duration;

use crate::{
    action::{ActionId, ActionRequest},
    params::Params,
    request::RequestId,
    TargetId,
};

use super::verification::{TcStateAccepted, TcStateStarted, VerificationToken};

use satrs_shared::res_code::ResultU16;
use spacepackets::{ecss::EcssEnumU16, time::UnixTimestamp};

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
        step: u16,
    },
    CompletionFailed {
        error_code: ResultU16,
        params: Params,
    },
    StepFailed {
        error_code: ResultU16,
        step: u16,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveActionRequest {
    action_id: ActionId,
    token: VerificationToken<TcStateStarted>,
    start_time: UnixTimestamp,
    timeout: Duration,
}

pub trait ActiveRequestMapProvider: Default {
    fn insert(&mut self, request_id: &RequestId, request: ActiveActionRequest);
    fn get(&self, request_id: RequestId) -> Option<ActiveActionRequest>;
    fn get_mut(&mut self, request_id: RequestId) -> Option<&mut ActiveActionRequest>;
    fn remove(&mut self, request_id: RequestId) -> bool;

    /// Call a user-supplied closure for each active request.
    fn for_each<F: FnMut(&RequestId, &ActiveActionRequest)>(&self, f: F);

    /// Call a user-supplied closure for each active request. Mutable variant.
    fn for_each_mut<F: FnMut(&RequestId, &mut ActiveActionRequest)>(&mut self, f: F);
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

    #[derive(Clone, Debug, Default)]
    pub struct DefaultActiveRequestMap(HashMap<RequestId, ActiveActionRequest>);

    impl ActiveRequestMapProvider for DefaultActiveRequestMap {
        // type Iter = hashbrown::hash_map::Iter<'a, RequestId, ActiveActionRequest>;
        // type IterMut = hashbrown::hash_map::IterMut<'a, RequestId, ActiveActionRequest>;

        fn insert(&mut self, request_id: &RequestId, request: ActiveActionRequest) {
            self.0.insert(*request_id, request);
        }

        fn get(&self, request_id: RequestId) -> Option<ActiveActionRequest> {
            self.0.get(&request_id).cloned()
        }

        fn get_mut(&mut self, request_id: RequestId) -> Option<&mut ActiveActionRequest> {
            self.0.get_mut(&request_id)
        }

        fn remove(&mut self, request_id: RequestId) -> bool {
            self.0.remove(&request_id).is_some()
        }

        fn for_each<F: FnMut(&RequestId, &ActiveActionRequest)>(&self, mut f: F) {
            for (req_id, active_req) in &self.0 {
                f(req_id, active_req);
            }
        }

        fn for_each_mut<F: FnMut(&RequestId, &mut ActiveActionRequest)>(&mut self, mut f: F) {
            for (req_id, active_req) in &mut self.0 {
                f(req_id, active_req);
            }
        }
    }

    pub trait ActionReplyHandlerHook {
        fn handle_unexpected_reply(&mut self, reply: &ActionReplyPusWithIds);
        fn timeout_callback(&self, active_request: &ActiveActionRequest);
        fn timeout_error_code(&self) -> ResultU16;
    }

    pub struct PusService8ReplyHandler<
        VerificationReporter: VerificationReportingProvider,
        ActiveRequestMap: ActiveRequestMapProvider,
        UserHook: ActionReplyHandlerHook,
    > {
        active_requests: ActiveRequestMap,
        verification_reporter: VerificationReporter,
        fail_data_buf: alloc::vec::Vec<u8>,
        current_time: UnixTimestamp,
        user_hook: UserHook,
    }

    impl<
            VerificationReporter: VerificationReportingProvider,
            ActiveRequestMap: ActiveRequestMapProvider,
            UserHook: ActionReplyHandlerHook,
        > PusService8ReplyHandler<VerificationReporter, ActiveRequestMap, UserHook>
    {
        #[cfg(feature = "std")]
        #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
        pub fn new_with_init_time_now(
            verification_reporter: VerificationReporter,
            fail_data_buf_size: usize,
            user_hook: UserHook,
        ) -> Result<Self, SystemTimeError> {
            let current_time = UnixTimestamp::from_now()?;
            Ok(Self::new(
                verification_reporter,
                fail_data_buf_size,
                user_hook,
                current_time,
            ))
        }

        pub fn new(
            verification_reporter: VerificationReporter,
            fail_data_buf_size: usize,
            user_hook: UserHook,
            init_time: UnixTimestamp,
        ) -> Self {
            Self {
                active_requests: ActiveRequestMap::default(),
                verification_reporter,
                fail_data_buf: alloc::vec![0; fail_data_buf_size],
                current_time: init_time,
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
                &request_id.into(),
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

        /// Check for timeouts across all active requests.
        ///
        /// It will call [Self::handle_timeout] for all active requests which have timed out.
        pub fn check_for_timeouts(&mut self, time_stamp: &[u8]) -> Result<(), EcssTmtcError> {
            let mut timed_out_commands = alloc::vec::Vec::new();
            self.active_requests.for_each(|request_id, active_req| {
                let diff = self.current_time - active_req.start_time;
                if diff.duration_absolute > active_req.timeout {
                    self.handle_timeout(active_req, time_stamp);
                }
                timed_out_commands.push(*request_id);
            });
            for timed_out_command in timed_out_commands {
                self.active_requests.remove(timed_out_command);
            }
            Ok(())
        }

        /// Handle the timeout for a given active request.
        ///
        /// This implementation will report a verification completion failure with a user-provided
        /// error code. It supplies the configured request timeout in milliseconds as a [u64]
        /// serialized in big-endian format as the failure data.
        pub fn handle_timeout(&self, active_request: &ActiveActionRequest, time_stamp: &[u8]) {
            let timeout = active_request.timeout.as_millis() as u64;
            let timeout_raw = timeout.to_be_bytes();
            self.verification_reporter
                .completion_failure(
                    active_request.token,
                    FailParams::new(
                        time_stamp,
                        &self.user_hook.timeout_error_code(),
                        &timeout_raw,
                    ),
                )
                .unwrap();
            self.user_hook.timeout_callback(active_request);
        }

        pub fn handle_action_reply(
            &mut self,
            action_reply_with_ids: ActionReplyPusWithIds,
            time_stamp: &[u8],
        ) -> Result<(), EcssTmtcError> {
            let active_req = self.active_requests.get(action_reply_with_ids.request_id);
            if active_req.is_none() {
                self.user_hook
                    .handle_unexpected_reply(&action_reply_with_ids);
            }
            let active_req = active_req.unwrap().clone();
            let remove_entry = match action_reply_with_ids.reply {
                ActionReplyPus::CompletionFailed { error_code, params } => {
                    let fail_data_len = params.write_to_be_bytes(&mut self.fail_data_buf)?;
                    self.verification_reporter
                        .completion_failure(
                            active_req.token,
                            FailParams::new(
                                time_stamp,
                                &error_code,
                                &self.fail_data_buf[..fail_data_len],
                            ),
                        )
                        .map_err(|e| e.0)?;
                    true
                }
                ActionReplyPus::StepFailed {
                    error_code,
                    step,
                    params,
                } => {
                    let fail_data_len = params.write_to_be_bytes(&mut self.fail_data_buf)?;
                    self.verification_reporter
                        .step_failure(
                            active_req.token,
                            FailParamsWithStep::new(
                                time_stamp,
                                &EcssEnumU16::new(step),
                                &error_code,
                                &self.fail_data_buf[..fail_data_len],
                            ),
                        )
                        .map_err(|e| e.0)?;
                    true
                }
                ActionReplyPus::Completed => {
                    self.verification_reporter
                        .completion_success(active_req.token, time_stamp)
                        .map_err(|e| e.0)?;
                    true
                }
                ActionReplyPus::StepSuccess { step } => {
                    self.verification_reporter.step_success(
                        &active_req.token,
                        time_stamp,
                        EcssEnumU16::new(step),
                    )?;
                    false
                }
            };
            if remove_entry {
                self.active_requests
                    .remove(action_reply_with_ids.request_id);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use core::{cell::RefCell, time::Duration};
    use std::time::SystemTimeError;

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
        params::{self, ParamsRaw, WritableToBeBytes},
        pus::{
            tests::{
                PusServiceHandlerWithVecCommon, PusTestHarness, SimplePusPacketHandler,
                TestConverter, TestRouter, TestRoutingErrorHandler, APP_DATA_TOO_SHORT, TEST_APID,
            },
            verification::{
                self,
                tests::{SharedVerificationMap, TestVerificationReporter, VerificationStatus},
                FailParams, TcStateNone, TcStateStarted, VerificationReportingProvider,
            },
            EcssTcInVecConverter, EcssTmtcError, GenericRoutingError, MpscTcReceiver,
            PusPacketHandlerResult, PusPacketHandlingError, TmAsVecSenderWithMpsc,
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

    struct Pus8RequestTestbenchWithVec {
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

    impl Pus8RequestTestbenchWithVec {
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

    impl PusTestHarness for Pus8RequestTestbenchWithVec {
        delegate! {
            to self.common {
                fn send_tc(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted>;
                fn read_next_tm(&mut self) -> PusTmReader<'_>;
                fn check_no_tm_available(&self) -> bool;
                fn check_next_verification_tm(
                    &self,
                    subservice: u8,
                    expected_request_id: verification::RequestId,
                );
            }
        }
    }
    impl SimplePusPacketHandler for Pus8RequestTestbenchWithVec {
        delegate! {
            to self.handler {
                fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError>;
            }
        }
    }

    const TIMEOUT_ERROR_CODE: ResultU16 = ResultU16::new(1, 2);
    const COMPLETION_ERROR_CODE: ResultU16 = ResultU16::new(2, 0);
    const COMPLETION_ERROR_CODE_STEP: ResultU16 = ResultU16::new(2, 1);

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

    pub struct Pus8ReplyTestbench {
        verif_reporter: TestVerificationReporter,
        handler: PusService8ReplyHandler<
            TestVerificationReporter,
            DefaultActiveRequestMap,
            TestReplyHandlerHook,
        >,
    }

    impl Pus8ReplyTestbench {
        pub fn new() -> Self {
            let reply_handler_hook = TestReplyHandlerHook::default();
            let shared_verif_map = SharedVerificationMap::default();
            let test_verif_reporter = TestVerificationReporter::new(shared_verif_map.clone());
            let reply_handler = PusService8ReplyHandler::new_with_init_time_now(
                test_verif_reporter.clone(),
                128,
                reply_handler_hook,
            )
            .expect("creating reply handler failed");
            Self {
                verif_reporter: test_verif_reporter,
                handler: reply_handler,
            }
        }

        pub fn init_handling_for_request(
            &mut self,
            request_id: RequestId,
            _action_id: ActionId,
        ) -> VerificationToken<TcStateStarted> {
            // let action_req = ActionRequest::new(action_id, ActionRequestVariant::NoData);
            let token = self.add_tc_with_req_id(request_id.into());
            let token = self
                .verif_reporter
                .acceptance_success(token, &[])
                .expect("acceptance success failure");
            let token = self
                .verif_reporter
                .start_success(token, &[])
                .expect("start success failure");
            let verif_info = self
                .verif_reporter
                .verification_info(&verification::RequestId::from(request_id))
                .expect("no verification info found");
            assert!(verif_info.started.expect("request was not started"));
            assert!(verif_info.accepted.expect("request was not accepted"));
            token
        }

        pub fn assert_request_completion_success(&self, step: Option<u16>, request_id: RequestId) {
            let verif_info = self
                .verif_reporter
                .verification_info(&verification::RequestId::from(request_id))
                .expect("no verification info found");
            self.assert_request_completion_common(&verif_info, step, true)
        }

        pub fn assert_request_completion_failure(
            &self,
            step: Option<u16>,
            request_id: RequestId,
            fail_enum: ResultU16,
            fail_data: &[u8],
        ) {
            let verif_info = self
                .verif_reporter
                .verification_info(&verification::RequestId::from(request_id))
                .expect("no verification info found");
            self.assert_request_completion_common(&verif_info, step, false);
            assert_eq!(verif_info.fail_enum.unwrap(), fail_enum.raw() as u64);
            assert_eq!(verif_info.failure_data.unwrap(), fail_data);
        }

        pub fn assert_request_completion_common(
            &self,
            verif_info: &VerificationStatus,
            step: Option<u16>,
            completion_success: bool,
        ) {
            if let Some(step) = step {
                assert!(verif_info.step_status.is_some());
                assert!(verif_info.step_status.unwrap());
                assert_eq!(step, verif_info.step);
            }
            assert_eq!(
                verif_info.completed.expect("request is not completed"),
                completion_success
            );
        }

        pub fn assert_request_step_failure(&self, step: u16, request_id: RequestId) {
            let verif_info = self
                .verif_reporter
                .verification_info(&verification::RequestId::from(request_id))
                .expect("no verification info found");
            assert!(verif_info.step_status.is_some());
            assert!(!verif_info.step_status.unwrap());
            assert_eq!(step, verif_info.step);
        }

        delegate! {
            to self.handler {
                pub fn handle_action_reply(
                    &mut self,
                    action_reply_with_ids: ActionReplyPusWithIds,
                    time_stamp: &[u8],
                ) -> Result<(), EcssTmtcError>;

                pub fn add_routed_request(
                    &mut self,
                    request_id: verification::RequestId,
                    action_id: ActionId,
                    token: VerificationToken<TcStateStarted>,
                    timeout: Duration,
                );

                pub fn update_time_from_now(&mut self) -> Result<(), SystemTimeError>;

                pub fn check_for_timeouts(&mut self, time_stamp: &[u8]) -> Result<(), EcssTmtcError>;
            }
            to self.verif_reporter {
                fn add_tc_with_req_id(&mut self, req_id: verification::RequestId) -> VerificationToken<TcStateNone>;
            }
        }
    }

    #[test]
    fn basic_test() {
        let mut action_handler = Pus8RequestTestbenchWithVec::new();
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
        let mut action_handler = Pus8RequestTestbenchWithVec::new();
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
    fn test_reply_handler_completion_success() {
        let mut reply_testbench = Pus8ReplyTestbench::new();
        let request_id = 0x02;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            action_id,
            token,
            Duration::from_millis(1),
        );
        let action_reply = ActionReplyPusWithIds {
            request_id,
            action_id,
            reply: ActionReplyPus::Completed,
        };
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        reply_testbench.assert_request_completion_success(None, request_id);
    }

    #[test]
    fn test_reply_handler_step_success() {
        let mut reply_testbench = Pus8ReplyTestbench::new();
        let request_id = 0x02;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            action_id,
            token,
            Duration::from_millis(1),
        );
        let action_reply = ActionReplyPusWithIds {
            request_id,
            action_id,
            reply: ActionReplyPus::StepSuccess { step: 1 },
        };
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        let action_reply = ActionReplyPusWithIds {
            request_id,
            action_id,
            reply: ActionReplyPus::Completed,
        };
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        reply_testbench.assert_request_completion_success(Some(1), request_id);
    }

    #[test]
    fn test_reply_handler_completion_failure() {
        let mut reply_testbench = Pus8ReplyTestbench::new();
        let request_id = 0x02;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            action_id,
            token,
            Duration::from_millis(1),
        );
        let params_raw = ParamsRaw::U32(params::U32(5));
        let action_reply = ActionReplyPusWithIds {
            request_id,
            action_id,
            reply: ActionReplyPus::CompletionFailed {
                error_code: COMPLETION_ERROR_CODE,
                params: params_raw.into(),
            },
        };
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        reply_testbench.assert_request_completion_failure(
            None,
            request_id,
            COMPLETION_ERROR_CODE,
            &params_raw.to_vec().unwrap(),
        );
    }

    #[test]
    fn test_reply_handler_step_failure() {
        let mut reply_testbench = Pus8ReplyTestbench::new();
        let request_id = 0x02;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            action_id,
            token,
            Duration::from_millis(1),
        );
        let action_reply = ActionReplyPusWithIds {
            request_id,
            action_id,
            reply: ActionReplyPus::StepFailed {
                error_code: COMPLETION_ERROR_CODE_STEP,
                step: 2,
                params: ParamsRaw::U32(crate::params::U32(5)).into(),
            },
        };
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        reply_testbench.assert_request_step_failure(2, request_id);
    }

    #[test]
    fn test_reply_handler_timeout_handling() {
        let mut reply_testbench = Pus8ReplyTestbench::new();
        let request_id = 0x02;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            action_id,
            token,
            Duration::from_millis(1),
        );
        let timeout_param = Duration::from_millis(1).as_millis() as u64;
        let timeout_param_raw = timeout_param.to_be_bytes();
        std::thread::sleep(Duration::from_millis(2));
        reply_testbench
            .update_time_from_now()
            .expect("time update failure");
        reply_testbench.check_for_timeouts(&[]).unwrap();
        reply_testbench.assert_request_completion_failure(
            None,
            request_id,
            TIMEOUT_ERROR_CODE,
            &timeout_param_raw,
        );
    }
}
