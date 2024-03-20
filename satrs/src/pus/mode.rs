use num_enum::{IntoPrimitive, TryFromPrimitive};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::mode::ModeReply;

#[cfg(feature = "alloc")]
#[allow(unused_imports)]
pub use alloc_mod::*;

#[cfg(feature = "std")]
pub use std_mod::*;

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

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod alloc_mod {}

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod std_mod {
    use core::time::Duration;

    use satrs_shared::res_code::ResultU16;
    use spacepackets::{
        ecss::tm::{PusTmCreator, PusTmSecondaryHeader},
        util::UnsignedEnum,
        SpHeader,
    };

    /*
    pub trait ModeReplyHook: ReplyHandlerHook<ActivePusRequest, ModeReply> {
        fn wrong_mode_result_code(&self) -> ResultU16;
        fn can_not_reach_mode_result_code(&self) -> ResultU16;
    }
    */

    use super::{ModeReply, MODE_SERVICE_ID};

    /*
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
        ActivePusRequest,
        ModeReply,
    >;

    impl<
            VerificationReporter: VerificationReportingProvider,
            ActiveRequestMap: ActiveRequestMapProvider<ActivePusRequest>,
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
                ActivePusRequest {
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
            mode_reply_with_id: &GenericModeReply,
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
                    reply.write_to_be_bytes(&mut self.tm_buf)?;
                    let req_id = verification::RequestId::from(mode_reply_with_id.request_id);
                    let mut sp_header = SpHeader::tm_unseg(
                        req_id.packet_id().apid(),
                        req_id.packet_seq_ctrl().seq_count(),
                        0,
                    )
                    .expect("space packet header creation error");
                    let sec_header = PusTmSecondaryHeader::new(
                        MODE_SERVICE_ID,
                        Subservice::TmModeReply as u8,
                        0,
                        0,
                        Some(time_stamp),
                    );
                    let pus_tm = PusTmCreator::new(&mut sp_header, sec_header, &self.tm_buf, true);
                    self.tm_sender.send_tm(PusTmWrapper::Direct(pus_tm))?;
                    self.verification_reporter
                        .completion_success(active_req.token, time_stamp)
                        .map_err(|e| e.0)?;
                    true
                }
                ModeReply::CantReachMode(reason) => {
                    let fail_data_len = reason.write_to_be_bytes(&mut self.tm_buf)?;
                    self.verification_reporter
                        .completion_failure(
                            active_req.token,
                            FailParams::new(
                                time_stamp,
                                &self.user_hook.can_not_reach_mode_result_code(),
                                &self.tm_buf[0..fail_data_len],
                            ),
                        )
                        .map_err(|e| e.0)?;
                    true
                }
                ModeReply::WrongMode { expected, reached } => {
                    let expected_len = expected.write_to_be_bytes(&mut self.tm_buf)?;
                    let reached_len =
                        reached.write_to_be_bytes(&mut self.tm_buf[expected_len..])?;
                    self.verification_reporter
                        .completion_failure(
                            active_req.token,
                            FailParams::new(
                                time_stamp,
                                &self.user_hook.can_not_reach_mode_result_code(),
                                &self.tm_buf[0..expected_len + reached_len],
                            ),
                        )
                        .map_err(|e| e.0)?;
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
    */
}

#[cfg(test)]
mod tests {

    use std::sync::mpsc;

    use crate::{
        mode::{
            ModeAndSubmode, ModeReplySender, ModeRequest, ModeRequestSender,
            ModeRequestorAndHandlerMpsc, ModeRequestorMpsc,
        },
        pus::mode::ModeReply,
        request::GenericMessage,
    };

    const TEST_CHANNEL_ID_0: u32 = 5;
    const TEST_CHANNEL_ID_1: u32 = 6;
    const TEST_CHANNEL_ID_2: u32 = 7;

    #[test]
    fn test_simple_mode_requestor() {
        let (reply_sender, reply_receiver) = mpsc::channel();
        let (request_sender, request_receiver) = mpsc::channel();
        let mut mode_requestor = ModeRequestorMpsc::new(TEST_CHANNEL_ID_0, reply_receiver);
        mode_requestor.add_message_target(TEST_CHANNEL_ID_1, request_sender);

        // Send a request and verify it arrives at the receiver.
        let request_id = 2;
        let sent_request = ModeRequest::ReadMode;
        mode_requestor
            .send_mode_request(request_id, TEST_CHANNEL_ID_1, sent_request)
            .expect("send failed");
        let request = request_receiver.recv().expect("recv failed");
        assert_eq!(request.request_id, 2);
        assert_eq!(request.sender_id, TEST_CHANNEL_ID_0);
        assert_eq!(request.message, sent_request);

        // Send a reply and verify it arrives at the requestor.
        let mode_reply = ModeReply::ModeReply(ModeAndSubmode::new(1, 5));
        reply_sender
            .send(GenericMessage::new(
                request_id,
                TEST_CHANNEL_ID_1,
                mode_reply,
            ))
            .expect("send failed");
        let reply = mode_requestor.try_recv_mode_reply().expect("recv failed");
        assert!(reply.is_some());
        let reply = reply.unwrap();
        assert_eq!(reply.sender_id, TEST_CHANNEL_ID_1);
        assert_eq!(reply.request_id, 2);
        assert_eq!(reply.message, mode_reply);
    }

    #[test]
    fn test_mode_requestor_and_request_handler_request_sending() {
        let (_reply_sender_to_connector, reply_receiver_of_connector) = mpsc::channel();
        let (_request_sender_to_connector, request_receiver_of_connector) = mpsc::channel();

        let (request_sender_to_channel_1, request_receiver_channel_1) = mpsc::channel();
        //let (reply_sender_to_channel_2, reply_receiver_channel_2) = mpsc::channel();
        let mut mode_connector = ModeRequestorAndHandlerMpsc::new(
            TEST_CHANNEL_ID_0,
            request_receiver_of_connector,
            reply_receiver_of_connector,
        );
        assert_eq!(
            ModeRequestSender::local_channel_id(&mode_connector),
            TEST_CHANNEL_ID_0
        );
        assert_eq!(
            ModeReplySender::local_channel_id(&mode_connector),
            TEST_CHANNEL_ID_0
        );
        assert_eq!(mode_connector.local_channel_id_generic(), TEST_CHANNEL_ID_0);

        mode_connector.add_request_target(TEST_CHANNEL_ID_1, request_sender_to_channel_1);

        // Send a request and verify it arrives at the receiver.
        let request_id = 2;
        let sent_request = ModeRequest::ReadMode;
        mode_connector
            .send_mode_request(request_id, TEST_CHANNEL_ID_1, sent_request)
            .expect("send failed");

        let request = request_receiver_channel_1.recv().expect("recv failed");
        assert_eq!(request.request_id, 2);
        assert_eq!(request.sender_id, TEST_CHANNEL_ID_0);
        assert_eq!(request.message, ModeRequest::ReadMode);
    }

    #[test]
    fn test_mode_requestor_and_request_handler_reply_sending() {
        let (_reply_sender_to_connector, reply_receiver_of_connector) = mpsc::channel();
        let (_request_sender_to_connector, request_receiver_of_connector) = mpsc::channel();

        let (reply_sender_to_channel_2, reply_receiver_channel_2) = mpsc::channel();
        let mut mode_connector = ModeRequestorAndHandlerMpsc::new(
            TEST_CHANNEL_ID_0,
            request_receiver_of_connector,
            reply_receiver_of_connector,
        );
        mode_connector.add_reply_target(TEST_CHANNEL_ID_2, reply_sender_to_channel_2);

        // Send a request and verify it arrives at the receiver.
        let request_id = 2;
        let sent_reply = ModeReply::ModeInfo(ModeAndSubmode::new(3, 5));
        mode_connector
            .send_mode_reply(request_id, TEST_CHANNEL_ID_2, sent_reply)
            .expect("send failed");
        let reply = reply_receiver_channel_2.recv().expect("recv failed");
        assert_eq!(reply.request_id, 2);
        assert_eq!(reply.sender_id, TEST_CHANNEL_ID_0);
        assert_eq!(reply.message, sent_reply);
    }

    #[test]
    fn test_mode_reply_handler() {}
}
