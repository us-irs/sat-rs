use num_enum::{IntoPrimitive, TryFromPrimitive};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "alloc")]
#[allow(unused_imports)]
pub use alloc_mod::*;

#[cfg(feature = "std")]
#[allow(unused_imports)]
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
pub mod alloc_mod {}

#[cfg(feature = "alloc")]
pub mod std_mod {}

#[cfg(test)]
mod tests {

    use std::sync::mpsc;

    use crate::{
        mode::{
            ModeAndSubmode, ModeReply, ModeReplySender, ModeRequest, ModeRequestSender,
            ModeRequestorAndHandlerMpsc, ModeRequestorOneChildMpsc,
        },
        request::{GenericMessage, MessageMetadata},
    };

    const TEST_COMPONENT_ID_0: u64 = 5;
    const TEST_COMPONENT_ID_1: u64 = 6;
    const TEST_COMPONENT_ID_2: u64 = 7;

    #[test]
    fn test_simple_mode_requestor() {
        let (reply_sender, reply_receiver) = mpsc::channel();
        let (request_sender, request_receiver) = mpsc::channel();
        let mut mode_requestor =
            ModeRequestorOneChildMpsc::new(TEST_COMPONENT_ID_0, reply_receiver);
        mode_requestor.add_message_target(TEST_COMPONENT_ID_1, request_sender);

        // Send a request and verify it arrives at the receiver.
        let request_id = 2;
        let sent_request = ModeRequest::ReadMode;
        mode_requestor
            .send_mode_request(request_id, TEST_COMPONENT_ID_1, sent_request)
            .expect("send failed");
        let request = request_receiver.recv().expect("recv failed");
        assert_eq!(request.request_id(), 2);
        assert_eq!(request.sender_id(), TEST_COMPONENT_ID_0);
        assert_eq!(request.message, sent_request);

        // Send a reply and verify it arrives at the requestor.
        let mode_reply = ModeReply::ModeReply(ModeAndSubmode::new(1, 5));
        reply_sender
            .send(GenericMessage::new(
                MessageMetadata::new(request_id, TEST_COMPONENT_ID_1),
                mode_reply,
            ))
            .expect("send failed");
        let reply = mode_requestor.try_recv_mode_reply().expect("recv failed");
        assert!(reply.is_some());
        let reply = reply.unwrap();
        assert_eq!(reply.sender_id(), TEST_COMPONENT_ID_1);
        assert_eq!(reply.request_id(), 2);
        assert_eq!(reply.message, mode_reply);
    }

    #[test]
    fn test_mode_requestor_and_request_handler_request_sending() {
        let (_reply_sender_to_connector, reply_receiver_of_connector) = mpsc::channel();
        let (_request_sender_to_connector, request_receiver_of_connector) = mpsc::channel();

        let (request_sender_to_channel_1, request_receiver_channel_1) = mpsc::channel();
        //let (reply_sender_to_channel_2, reply_receiver_channel_2) = mpsc::channel();
        let mut mode_connector = ModeRequestorAndHandlerMpsc::new(
            TEST_COMPONENT_ID_0,
            request_receiver_of_connector,
            reply_receiver_of_connector,
        );
        assert_eq!(
            ModeRequestSender::local_channel_id(&mode_connector),
            TEST_COMPONENT_ID_0
        );
        assert_eq!(
            ModeReplySender::local_channel_id(&mode_connector),
            TEST_COMPONENT_ID_0
        );
        assert_eq!(
            mode_connector.local_channel_id_generic(),
            TEST_COMPONENT_ID_0
        );

        mode_connector.add_request_target(TEST_COMPONENT_ID_1, request_sender_to_channel_1);

        // Send a request and verify it arrives at the receiver.
        let request_id = 2;
        let sent_request = ModeRequest::ReadMode;
        mode_connector
            .send_mode_request(request_id, TEST_COMPONENT_ID_1, sent_request)
            .expect("send failed");

        let request = request_receiver_channel_1.recv().expect("recv failed");
        assert_eq!(request.request_id(), 2);
        assert_eq!(request.sender_id(), TEST_COMPONENT_ID_0);
        assert_eq!(request.message, ModeRequest::ReadMode);
    }

    #[test]
    fn test_mode_requestor_and_request_handler_reply_sending() {
        let (_reply_sender_to_connector, reply_receiver_of_connector) = mpsc::channel();
        let (_request_sender_to_connector, request_receiver_of_connector) = mpsc::channel();

        let (reply_sender_to_channel_2, reply_receiver_channel_2) = mpsc::channel();
        let mut mode_connector = ModeRequestorAndHandlerMpsc::new(
            TEST_COMPONENT_ID_0,
            request_receiver_of_connector,
            reply_receiver_of_connector,
        );
        mode_connector.add_reply_target(TEST_COMPONENT_ID_2, reply_sender_to_channel_2);

        // Send a reply and verify it arrives at the receiver.
        let request_id = 2;
        let sent_reply = ModeReply::ModeReply(ModeAndSubmode::new(3, 5));
        mode_connector
            .send_mode_reply(
                MessageMetadata::new(request_id, TEST_COMPONENT_ID_2),
                sent_reply,
            )
            .expect("send failed");
        let reply = reply_receiver_channel_2.recv().expect("recv failed");
        assert_eq!(reply.request_id(), 2);
        assert_eq!(reply.sender_id(), TEST_COMPONENT_ID_0);
        assert_eq!(reply.message, sent_reply);
    }

    #[test]
    fn test_mode_reply_handler() {}
}
