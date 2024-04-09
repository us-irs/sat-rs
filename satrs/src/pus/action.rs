use crate::{
    action::{ActionId, ActionRequest},
    params::Params,
    request::{GenericMessage, MessageMetadata, RequestId},
};

use satrs_shared::res_code::ResultU16;

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub use std_mod::*;

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
#[allow(unused_imports)]
pub use alloc_mod::*;

#[derive(Clone, Debug)]
pub struct ActionRequestWithId {
    pub request_id: RequestId,
    pub request: ActionRequest,
}

/// A reply to an action request, but tailored to the PUS standard verification process.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum ActionReplyVariant {
    Completed,
    StepSuccess {
        step: u16,
    },
    CompletionFailed {
        error_code: ResultU16,
        params: Option<Params>,
    },
    StepFailed {
        error_code: ResultU16,
        step: u16,
        params: Option<Params>,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub struct PusActionReply {
    pub action_id: ActionId,
    pub variant: ActionReplyVariant,
}

impl PusActionReply {
    pub fn new(action_id: ActionId, variant: ActionReplyVariant) -> Self {
        Self { action_id, variant }
    }
}

pub type GenericActionReplyPus = GenericMessage<PusActionReply>;

impl GenericActionReplyPus {
    pub fn new_action_reply(
        requestor_info: MessageMetadata,
        action_id: ActionId,
        reply: ActionReplyVariant,
    ) -> Self {
        Self::new(requestor_info, PusActionReply::new(action_id, reply))
    }
}

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod alloc_mod {
    use crate::{
        action::ActionRequest,
        queue::GenericTargetedMessagingError,
        request::{
            GenericMessage, MessageReceiver, MessageSender, MessageSenderAndReceiver, RequestId,
        },
        ComponentId,
    };

    use super::PusActionReply;

    /// Helper type definition for a mode handler which can handle mode requests.
    pub type ActionRequestHandlerInterface<S, R> =
        MessageSenderAndReceiver<PusActionReply, ActionRequest, S, R>;

    impl<S: MessageSender<PusActionReply>, R: MessageReceiver<ActionRequest>>
        ActionRequestHandlerInterface<S, R>
    {
        pub fn try_recv_action_request(
            &self,
        ) -> Result<Option<GenericMessage<ActionRequest>>, GenericTargetedMessagingError> {
            self.try_recv_message()
        }

        pub fn send_action_reply(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            reply: PusActionReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_id, reply)
        }
    }

    /// Helper type defintion for a mode handler object which can send mode requests and receive
    /// mode replies.
    pub type ActionRequestorInterface<S, R> =
        MessageSenderAndReceiver<ActionRequest, PusActionReply, S, R>;

    impl<S: MessageSender<ActionRequest>, R: MessageReceiver<PusActionReply>>
        ActionRequestorInterface<S, R>
    {
        pub fn try_recv_action_reply(
            &self,
        ) -> Result<Option<GenericMessage<PusActionReply>>, GenericTargetedMessagingError> {
            self.try_recv_message()
        }

        pub fn send_action_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            request: ActionRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_id, request)
        }
    }
}

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub mod std_mod {
    use std::sync::mpsc;

    use crate::{
        pus::{
            verification::{self, TcStateToken},
            ActivePusRequestStd, ActiveRequestProvider, DefaultActiveRequestMap,
        },
        ComponentId,
    };

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ActivePusActionRequestStd {
        pub action_id: ActionId,
        common: ActivePusRequestStd,
    }

    impl ActiveRequestProvider for ActivePusActionRequestStd {
        delegate::delegate! {
            to self.common {
                fn target_id(&self) -> ComponentId;
                fn token(&self) -> verification::TcStateToken;
                fn set_token(&mut self, token: verification::TcStateToken);
                fn has_timed_out(&self) -> bool;
                fn timeout(&self) -> core::time::Duration;
            }
        }
    }

    impl ActivePusActionRequestStd {
        pub fn new_from_common_req(action_id: ActionId, common: ActivePusRequestStd) -> Self {
            Self { action_id, common }
        }

        pub fn new(
            action_id: ActionId,
            target_id: ComponentId,
            token: TcStateToken,
            timeout: core::time::Duration,
        ) -> Self {
            Self {
                action_id,
                common: ActivePusRequestStd::new(target_id, token, timeout),
            }
        }
    }
    pub type DefaultActiveActionRequestMap = DefaultActiveRequestMap<ActivePusActionRequestStd>;

    pub type ActionRequestHandlerMpsc = ActionRequestHandlerInterface<
        mpsc::Sender<GenericMessage<PusActionReply>>,
        mpsc::Receiver<GenericMessage<ActionRequest>>,
    >;
    pub type ActionRequestHandlerMpscBounded = ActionRequestHandlerInterface<
        mpsc::SyncSender<GenericMessage<PusActionReply>>,
        mpsc::Receiver<GenericMessage<ActionRequest>>,
    >;

    pub type ActionRequestorMpsc = ActionRequestorInterface<
        mpsc::Sender<GenericMessage<ActionRequest>>,
        mpsc::Receiver<GenericMessage<PusActionReply>>,
    >;
    pub type ActionRequestorBoundedMpsc = ActionRequestorInterface<
        mpsc::SyncSender<GenericMessage<ActionRequest>>,
        mpsc::Receiver<GenericMessage<PusActionReply>>,
    >;

    /*
    pub type ModeRequestorAndHandlerMpsc = ModeInterface<
        mpsc::Sender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
        mpsc::Sender<GenericMessage<ModeReply>>,
        mpsc::Receiver<GenericMessage<ModeRequest>>,
    >;
    pub type ModeRequestorAndHandlerMpscBounded = ModeInterface<
        mpsc::SyncSender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
        mpsc::SyncSender<GenericMessage<ModeReply>>,
        mpsc::Receiver<GenericMessage<ModeRequest>>,
    >;
    */
}

#[cfg(test)]
mod tests {
    /*
    use core::{cell::RefCell, time::Duration};
    use std::{sync::mpsc, time::SystemTimeError};

    use alloc::{collections::VecDeque, vec::Vec};
    use delegate::delegate;

    use spacepackets::{
        ecss::{
            tc::{PusTcCreator, PusTcReader},
            tm::PusTmReader,
            PusPacket,
        },
        time::{cds, TimeWriter},
        CcsdsPacket,
    };

    use crate::{
        action::ActionRequestVariant,
        params::{self, ParamsRaw, WritableToBeBytes},
        pus::{
            tests::{
                PusServiceHandlerWithVecCommon, PusTestHarness, SimplePusPacketHandler,
                TestConverter, TestRouter, APP_DATA_TOO_SHORT,
            },
            verification::{
                self,
                tests::{SharedVerificationMap, TestVerificationReporter, VerificationStatus},
                FailParams, TcStateAccepted, TcStateNone, TcStateStarted,
                VerificationReportingProvider,
            },
            EcssTcInMemConverter, EcssTcInVecConverter, EcssTmtcError, GenericRoutingError,
            MpscTcReceiver, PusPacketHandlerResult, PusPacketHandlingError, PusRequestRouter,
            PusServiceHelper, PusTcToRequestConverter, TmAsVecSenderWithMpsc,
        },
    };

    use super::*;

    impl<Request> PusRequestRouter<Request> for TestRouter<Request> {
        type Error = GenericRoutingError;

        fn route(
            &self,
            target_id: TargetId,
            request: Request,
            _token: VerificationToken<TcStateAccepted>,
        ) -> Result<(), Self::Error> {
            self.routing_requests
                .borrow_mut()
                .push_back((target_id, request));
            self.check_for_injected_error()
        }

        fn handle_error(
            &self,
            target_id: TargetId,
            token: VerificationToken<TcStateAccepted>,
            tc: &PusTcReader,
            error: Self::Error,
            time_stamp: &[u8],
            verif_reporter: &impl VerificationReportingProvider,
        ) {
            self.routing_errors
                .borrow_mut()
                .push_back((target_id, error));
        }
    }

    impl PusTcToRequestConverter<ActionRequest> for TestConverter<8> {
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

    pub struct PusDynRequestHandler<const SERVICE: u8, Request> {
        srv_helper: PusServiceHelper<
            MpscTcReceiver,
            TmAsVecSenderWithMpsc,
            EcssTcInVecConverter,
            TestVerificationReporter,
        >,
        request_converter: TestConverter<SERVICE>,
        request_router: TestRouter<Request>,
    }

    struct Pus8RequestTestbenchWithVec {
        common: PusServiceHandlerWithVecCommon<TestVerificationReporter>,
        handler: PusDynRequestHandler<8, ActionRequest>,
    }

    impl Pus8RequestTestbenchWithVec {
        pub fn new() -> Self {
            let (common, srv_helper) = PusServiceHandlerWithVecCommon::new_with_test_verif_sender();
            Self {
                common,
                handler: PusDynRequestHandler {
                    srv_helper,
                    request_converter: TestConverter::default(),
                    request_router: TestRouter::default(),
                },
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
            to self.handler.request_router {
                pub fn retrieve_next_routing_error(&mut self) -> (TargetId, GenericRoutingError);
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
        fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
            let possible_packet = self.handler.srv_helper.retrieve_and_accept_next_packet()?;
            if possible_packet.is_none() {
                return Ok(PusPacketHandlerResult::Empty);
            }
            let ecss_tc_and_token = possible_packet.unwrap();
            let tc = self
                .handler
                .srv_helper
                .tc_in_mem_converter
                .convert_ecss_tc_in_memory_to_reader(&ecss_tc_and_token.tc_in_memory)?;
            let time_stamp = cds::TimeProvider::from_now_with_u16_days()
                .expect("timestamp generation failed")
                .to_vec()
                .unwrap();
            let (target_id, action_request) = self.handler.request_converter.convert(
                ecss_tc_and_token.token,
                &tc,
                &time_stamp,
                &self.handler.srv_helper.common.verification_handler,
            )?;
            if let Err(e) = self.handler.request_router.route(
                target_id,
                action_request,
                ecss_tc_and_token.token,
            ) {
                self.handler.request_router.handle_error(
                    target_id,
                    ecss_tc_and_token.token,
                    &tc,
                    e.clone(),
                    &time_stamp,
                    &self.handler.srv_helper.common.verification_handler,
                );
                return Err(e.into());
            }
            Ok(PusPacketHandlerResult::RequestHandled)
        }
    }

    const TIMEOUT_ERROR_CODE: ResultU16 = ResultU16::new(1, 2);
    const COMPLETION_ERROR_CODE: ResultU16 = ResultU16::new(2, 0);
    const COMPLETION_ERROR_CODE_STEP: ResultU16 = ResultU16::new(2, 1);

    #[derive(Default)]
    pub struct TestReplyHandlerHook {
        pub unexpected_replies: VecDeque<GenericActionReplyPus>,
        pub timeouts: RefCell<VecDeque<ActivePusActionRequest>>,
    }

    impl ReplyHandlerHook<ActivePusActionRequest, ActionReplyPusWithActionId> for TestReplyHandlerHook {
        fn handle_unexpected_reply(&mut self, reply: &GenericActionReplyPus) {
            self.unexpected_replies.push_back(reply.clone());
        }

        fn timeout_callback(&self, active_request: &ActivePusActionRequest) {
            self.timeouts.borrow_mut().push_back(active_request.clone());
        }

        fn timeout_error_code(&self) -> ResultU16 {
            TIMEOUT_ERROR_CODE
        }
    }

    pub struct Pus8ReplyTestbench {
        verif_reporter: TestVerificationReporter,
        #[allow(dead_code)]
        ecss_tm_receiver: mpsc::Receiver<Vec<u8>>,
        handler: PusService8ReplyHandler<
            TestVerificationReporter,
            DefaultActiveActionRequestMap,
            TestReplyHandlerHook,
            mpsc::Sender<Vec<u8>>,
        >,
    }

    impl Pus8ReplyTestbench {
        pub fn new(normal_ctor: bool) -> Self {
            let reply_handler_hook = TestReplyHandlerHook::default();
            let shared_verif_map = SharedVerificationMap::default();
            let test_verif_reporter = TestVerificationReporter::new(shared_verif_map.clone());
            let (ecss_tm_sender, ecss_tm_receiver) = mpsc::channel();
            let reply_handler = if normal_ctor {
                PusService8ReplyHandler::new_from_now_with_default_map(
                    test_verif_reporter.clone(),
                    128,
                    reply_handler_hook,
                    ecss_tm_sender,
                )
                .expect("creating reply handler failed")
            } else {
                PusService8ReplyHandler::new_from_now(
                    test_verif_reporter.clone(),
                    DefaultActiveActionRequestMap::default(),
                    128,
                    reply_handler_hook,
                    ecss_tm_sender,
                )
                .expect("creating reply handler failed")
            };
            Self {
                verif_reporter: test_verif_reporter,
                ecss_tm_receiver,
                handler: reply_handler,
            }
        }

        pub fn init_handling_for_request(
            &mut self,
            request_id: RequestId,
            _action_id: ActionId,
        ) -> VerificationToken<TcStateStarted> {
            assert!(!self.handler.request_active(request_id));
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

        pub fn next_unrequested_reply(&self) -> Option<GenericActionReplyPus> {
            self.handler.user_hook.unexpected_replies.front().cloned()
        }

        pub fn assert_request_completion_success(&self, step: Option<u16>, request_id: RequestId) {
            let verif_info = self
                .verif_reporter
                .verification_info(&verification::RequestId::from(request_id))
                .expect("no verification info found");
            self.assert_request_completion_common(request_id, &verif_info, step, true);
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
            self.assert_request_completion_common(request_id, &verif_info, step, false);
            assert_eq!(verif_info.fail_enum.unwrap(), fail_enum.raw() as u64);
            assert_eq!(verif_info.failure_data.unwrap(), fail_data);
        }

        pub fn assert_request_completion_common(
            &self,
            request_id: RequestId,
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
            assert!(!self.handler.request_active(request_id));
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
        pub fn add_routed_request(
            &mut self,
            request_id: verification::RequestId,
            target_id: TargetId,
            action_id: ActionId,
            token: VerificationToken<TcStateStarted>,
            timeout: Duration,
        ) {
            if self.handler.request_active(request_id.into()) {
                panic!("request already present");
            }
            self.handler
                .add_routed_action_request(request_id, target_id, action_id, token, timeout);
            if !self.handler.request_active(request_id.into()) {
                panic!("request should be active now");
            }
        }

        delegate! {
            to self.handler {
                pub fn request_active(&self, request_id: RequestId) -> bool;

        pub fn handle_action_reply(
            &mut self,
            action_reply_with_ids: GenericMessage<ActionReplyPusWithActionId>,
            time_stamp: &[u8]
        ) -> Result<(), EcssTmtcError>;

                pub fn update_time_from_now(&mut self) -> Result<(), SystemTimeError>;

                pub fn check_for_timeouts(&mut self, time_stamp: &[u8]) -> Result<(), EcssTmtcError>;
            }
            to self.verif_reporter {
                fn add_tc_with_req_id(&mut self, req_id: verification::RequestId) -> VerificationToken<TcStateNone>;
            }
        }
    }

    #[test]
    fn test_reply_handler_completion_success() {
        let mut reply_testbench = Pus8ReplyTestbench::new(true);
        let sender_id = 0x06;
        let request_id = 0x02;
        let target_id = 0x05;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            target_id,
            action_id,
            token,
            Duration::from_millis(1),
        );
        assert!(reply_testbench.request_active(request_id));
        let action_reply = GenericMessage::new(
            request_id,
            sender_id,
            ActionReplyPusWithActionId {
                action_id,
                variant: ActionReplyPus::Completed,
            },
        );
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        reply_testbench.assert_request_completion_success(None, request_id);
    }

    #[test]
    fn test_reply_handler_step_success() {
        let mut reply_testbench = Pus8ReplyTestbench::new(false);
        let request_id = 0x02;
        let target_id = 0x05;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            target_id,
            action_id,
            token,
            Duration::from_millis(1),
        );
        let action_reply = GenericActionReplyPus::new_action_reply(
            request_id,
            action_id,
            action_id,
            ActionReplyPus::StepSuccess { step: 1 },
        );
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        let action_reply = GenericActionReplyPus::new_action_reply(
            request_id,
            action_id,
            action_id,
            ActionReplyPus::Completed,
        );
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        reply_testbench.assert_request_completion_success(Some(1), request_id);
    }

    #[test]
    fn test_reply_handler_completion_failure() {
        let mut reply_testbench = Pus8ReplyTestbench::new(true);
        let sender_id = 0x01;
        let request_id = 0x02;
        let target_id = 0x05;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            target_id,
            action_id,
            token,
            Duration::from_millis(1),
        );
        let params_raw = ParamsRaw::U32(params::U32(5));
        let action_reply = GenericActionReplyPus::new_action_reply(
            request_id,
            sender_id,
            action_id,
            ActionReplyPus::CompletionFailed {
                error_code: COMPLETION_ERROR_CODE,
                params: params_raw.into(),
            },
        );
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
        let mut reply_testbench = Pus8ReplyTestbench::new(false);
        let sender_id = 0x01;
        let request_id = 0x02;
        let target_id = 0x05;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            target_id,
            action_id,
            token,
            Duration::from_millis(1),
        );
        let action_reply = GenericActionReplyPus::new_action_reply(
            request_id,
            sender_id,
            action_id,
            ActionReplyPus::StepFailed {
                error_code: COMPLETION_ERROR_CODE_STEP,
                step: 2,
                params: ParamsRaw::U32(crate::params::U32(5)).into(),
            },
        );
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        reply_testbench.assert_request_step_failure(2, request_id);
    }

    #[test]
    fn test_reply_handler_timeout_handling() {
        let mut reply_testbench = Pus8ReplyTestbench::new(true);
        let request_id = 0x02;
        let target_id = 0x06;
        let action_id = 0x03;
        let token = reply_testbench.init_handling_for_request(request_id, action_id);
        reply_testbench.add_routed_request(
            request_id.into(),
            target_id,
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

    #[test]
    fn test_unrequested_reply() {
        let mut reply_testbench = Pus8ReplyTestbench::new(true);
        let sender_id = 0x01;
        let request_id = 0x02;
        let action_id = 0x03;

        let action_reply = GenericActionReplyPus::new_action_reply(
            request_id,
            sender_id,
            action_id,
            ActionReplyPus::Completed,
        );
        reply_testbench
            .handle_action_reply(action_reply, &[])
            .expect("reply handling failure");
        let reply = reply_testbench.next_unrequested_reply();
        assert!(reply.is_some());
        let reply = reply.unwrap();
        assert_eq!(reply.message.action_id, action_id);
        assert_eq!(reply.request_id, request_id);
        assert_eq!(reply.message.variant, ActionReplyPus::Completed);
    }
    */
}
