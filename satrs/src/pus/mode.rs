use num_enum::{IntoPrimitive, TryFromPrimitive};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{mode::ModeReply, request::GenericMessage};

pub const MODE_SERVICE_ID: u8 = 200;

#[derive(Debug, Eq, PartialEq, Copy, Clone, IntoPrimitive, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(u8)]
pub enum Subservice {
    TcSetMode = 1,
    TcReadMode = 3,
    TcAnnounceMode = 4,
    TcAnnounceModeRecursive = 5,
    TmModeReply = 6,
    TmCantReachMode = 7,
    TmWrongModeReply = 8,
}

pub type GenericModeReplyPus = GenericMessage<ModeReply>;

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod alloc_mod {}

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod std_mod {
    use core::time::Duration;

    use satrs_shared::res_code::ResultU16;

    use crate::{
        mode::{ModeReply, ModeRequest},
        pus::{
            verification::{
                self, FailParams, TcStateStarted, VerificationReportingProvider, VerificationToken,
            },
            ActiveRequest, ActiveRequestMapProvider, EcssTmSenderCore, EcssTmtcError,
            GenericRoutingError, PusServiceReplyHandler, PusTargetedRequestHandler,
            ReplyHandlerHook,
        },
        TargetId,
    };

    pub trait ModeReplyHook: ReplyHandlerHook<ActiveRequest, ModeReply> {
        fn wrong_mode_result_code(&self) -> ResultU16;
        fn can_not_reach_mode_result_code(&self) -> ResultU16;
    }

    use super::GenericModeReplyPus;

    pub type PusModeServiceRequestHandler<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        RequestConverter,
        RequestRouter,
        RoutingErrorHandler,
        RoutingError = GenericRoutingError,
    > = PusTargetedRequestHandler<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        RequestConverter,
        RequestRouter,
        RoutingErrorHandler,
        ModeRequest,
        RoutingError,
    >;

    /// Type definition for a PUS mode servicd reply handler which constrains the
    /// [PusServiceReplyHandler] active request and reply generics to the [ActiveActionRequest] and
    /// [ActionReplyPusWithIds] type.
    pub type PusModeServiceReplyHandler<
        VerificationReporter,
        ActiveRequestMap,
        UserHook,
        TmSender,
    > = PusServiceReplyHandler<
        VerificationReporter,
        ActiveRequestMap,
        UserHook,
        TmSender,
        ActiveRequest,
        ModeReply,
    >;

    impl<
            VerificationReporter: VerificationReportingProvider,
            ActiveRequestMap: ActiveRequestMapProvider<ActiveRequest>,
            UserHook: ModeReplyHook,
            TmSender: EcssTmSenderCore,
        > PusModeServiceReplyHandler<VerificationReporter, ActiveRequestMap, UserHook, TmSender>
    {
        /// Helper method to register a recently routed action request.
        pub fn add_routed_mode_request(
            &mut self,
            request_id: verification::RequestId,
            target_id: TargetId,
            token: VerificationToken<TcStateStarted>,
            timeout: Duration,
        ) {
            self.active_request_map.insert(
                &request_id.into(),
                ActiveRequest {
                    target_id,
                    token,
                    start_time: self.current_time,
                    timeout,
                },
            )
        }

        /// Main handler function to handle all received action replies.
        pub fn handle_mode_reply(
            &mut self,
            mode_reply_with_id: &GenericModeReplyPus,
            time_stamp: &[u8],
        ) -> Result<(), EcssTmtcError> {
            let active_req = self.active_request_map.get(mode_reply_with_id.request_id);
            if active_req.is_none() {
                self.user_hook.handle_unexpected_reply(mode_reply_with_id);
                return Ok(());
            }
            let active_req = active_req.unwrap().clone();
            let remove_entry = match mode_reply_with_id.message {
                ModeReply::ModeReply(reply) => {
                    // TODO: Send dedicated TM to send mode information.
                    // TODO: Generate TM. Service ID from hook, subservice ID is fixed for
                    // framework purposes, APID can be retrieved from Request ID.

                    self.verification_reporter
                        .completion_success(active_req.token, time_stamp)
                        .map_err(|e| e.0)?;
                    true
                }
                ModeReply::CantReachMode(reached_mode) => {
                    let fail_data_len = reached_mode.to_be_bytes(&mut self.fail_data_buf)?;
                    self.verification_reporter
                        .completion_failure(
                            active_req.token,
                            FailParams::new(
                                time_stamp,
                                &self.user_hook.can_not_reach_mode_result_code(),
                                &self.fail_data_buf[0..fail_data_len],
                            ),
                        )
                        .map_err(|e| e.0)?;
                    true
                }
                ModeReply::WrongMode { expected, reached } => {
                    // TODO: Generate completion failure with appropriate result code and reached
                    // mode as context information.
                    // self.verification_reporter.completion_success(active_req.token, time_stamp);
                    true
                }
                _ => true,
            };
            if remove_entry {
                self.active_request_map
                    .remove(mode_reply_with_id.request_id);
            }
            Ok(())
        }
    }
}
