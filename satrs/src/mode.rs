use core::mem::size_of;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use spacepackets::ByteConversionError;

#[cfg(feature = "alloc")]
pub use alloc_mod::*;

#[cfg(feature = "std")]
pub use std_mod::*;

use crate::{
    queue::GenericTargetedMessagingError,
    request::{GenericMessage, MessageReceiver, MessageReceiverWithId, RequestId},
    ChannelId, TargetId,
};

pub type Mode = u32;
pub type Submode = u16;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModeAndSubmode {
    mode: Mode,
    submode: Submode,
}

impl ModeAndSubmode {
    pub const fn new_mode_only(mode: Mode) -> Self {
        Self { mode, submode: 0 }
    }

    pub const fn new(mode: Mode, submode: Submode) -> Self {
        Self { mode, submode }
    }

    pub fn raw_len() -> usize {
        size_of::<u32>() + size_of::<u16>()
    }

    pub fn from_be_bytes(buf: &[u8]) -> Result<Self, ByteConversionError> {
        if buf.len() < 6 {
            return Err(ByteConversionError::FromSliceTooSmall {
                expected: 6,
                found: buf.len(),
            });
        }
        Ok(Self {
            mode: Mode::from_be_bytes(buf[0..size_of::<Mode>()].try_into().unwrap()),
            submode: Submode::from_be_bytes(
                buf[size_of::<Mode>()..size_of::<Mode>() + size_of::<Submode>()]
                    .try_into()
                    .unwrap(),
            ),
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn submode(&self) -> Submode {
        self.submode
    }
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TargetedModeCommand {
    pub address: TargetId,
    pub mode_submode: ModeAndSubmode,
}

impl TargetedModeCommand {
    pub const fn new(address: TargetId, mode_submode: ModeAndSubmode) -> Self {
        Self {
            address,
            mode_submode,
        }
    }

    pub fn address(&self) -> TargetId {
        self.address
    }

    pub fn mode_submode(&self) -> ModeAndSubmode {
        self.mode_submode
    }

    pub fn mode(&self) -> u32 {
        self.mode_submode.mode
    }

    pub fn submode(&self) -> u16 {
        self.mode_submode.submode
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ModeRequest {
    SetMode(ModeAndSubmode),
    ReadMode,
    AnnounceMode,
    AnnounceModeRecursive,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ModeReply {
    /// Unrequest mode information. Can be used to notify other components of changed modes.
    ModeInfo(ModeAndSubmode),
    /// Reply to a mode request to confirm the commanded mode was reached.
    ModeReply(ModeAndSubmode),
    CantReachMode(ModeAndSubmode),
    WrongMode {
        expected: ModeAndSubmode,
        reached: ModeAndSubmode,
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TargetedModeRequest {
    target_id: TargetId,
    mode_request: ModeRequest,
}

pub trait ModeRequestSender {
    fn local_channel_id(&self) -> ChannelId;
    fn send_mode_request(
        &self,
        request_id: RequestId,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError>;
}

pub trait ModeReplySender {
    fn local_channel_id(&self) -> ChannelId;

    fn send_mode_reply(
        &self,
        request_id: RequestId,
        target_id: ChannelId,
        reply: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError>;
}

pub trait ModeRequestReceiver {
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<GenericMessage<ModeRequest>>, GenericTargetedMessagingError>;
}

pub trait ModeReplyReceiver {
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<GenericMessage<ModeReply>>, GenericTargetedMessagingError>;
}

impl<R: MessageReceiver<ModeReply>> ModeReplyReceiver for MessageReceiverWithId<ModeReply, R> {
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<GenericMessage<ModeReply>>, GenericTargetedMessagingError> {
        self.try_recv_message()
    }
}

impl<R: MessageReceiver<ModeRequest>> ModeRequestReceiver
    for MessageReceiverWithId<ModeRequest, R>
{
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<GenericMessage<ModeRequest>>, GenericTargetedMessagingError> {
        self.try_recv_message()
    }
}

pub trait ModeProvider {
    fn mode_and_submode(&self) -> ModeAndSubmode;
}

#[derive(Debug, Clone)]
pub enum ModeError {
    Messaging(GenericTargetedMessagingError),
}

impl From<GenericTargetedMessagingError> for ModeError {
    fn from(value: GenericTargetedMessagingError) -> Self {
        Self::Messaging(value)
    }
}

pub trait ModeRequestHandler: ModeProvider {
    fn start_transition(
        &mut self,
        request_id: RequestId,
        sender_id: ChannelId,
        mode_and_submode: ModeAndSubmode,
    ) -> Result<(), ModeError>;

    fn announce_mode(&self, request_id: RequestId, sender_id: ChannelId, recursive: bool);
    fn handle_mode_reached(&mut self) -> Result<(), GenericTargetedMessagingError>;
}

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod alloc_mod {
    use crate::request::{
        MessageSender, MessageSenderAndReceiver, MessageSenderMap, MessageSenderMapWithId,
        RequestAndReplySenderAndReceiver,
    };

    use super::*;

    impl<S: MessageSender<ModeRequest>> MessageSenderMap<ModeRequest, S> {
        pub fn send_mode_request(
            &self,
            request_id: RequestId,
            local_id: ChannelId,
            target_id: ChannelId,
            request: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, local_id, target_id, request)
        }

        pub fn add_request_target(&mut self, target_id: ChannelId, request_sender: S) {
            self.add_message_target(target_id, request_sender)
        }
    }

    impl<S: MessageSender<ModeReply>> MessageSenderMap<ModeReply, S> {
        pub fn send_mode_reply(
            &self,
            request_id: RequestId,
            local_id: ChannelId,
            target_id: ChannelId,
            request: ModeReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, local_id, target_id, request)
        }

        pub fn add_reply_target(&mut self, target_id: ChannelId, request_sender: S) {
            self.add_message_target(target_id, request_sender)
        }
    }

    impl<S: MessageSender<ModeReply>> ModeReplySender for MessageSenderMapWithId<ModeReply, S> {
        fn send_mode_reply(
            &self,
            request_id: RequestId,
            target_channel_id: ChannelId,
            reply: ModeReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_channel_id, reply)
        }

        fn local_channel_id(&self) -> ChannelId {
            self.local_channel_id
        }
    }

    impl<S: MessageSender<ModeRequest>> ModeRequestSender for MessageSenderMapWithId<ModeRequest, S> {
        fn local_channel_id(&self) -> ChannelId {
            self.local_channel_id
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ChannelId,
            request: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_id, request)
        }
    }

    impl<FROM, S: MessageSender<ModeReply>, R: MessageReceiver<FROM>> ModeReplySender
        for MessageSenderAndReceiver<ModeReply, FROM, S, R>
    {
        fn local_channel_id(&self) -> ChannelId {
            self.local_channel_id_generic()
        }

        fn send_mode_reply(
            &self,
            request_id: RequestId,
            target_id: ChannelId,
            request: ModeReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.message_sender_map.send_mode_reply(
                request_id,
                self.local_channel_id(),
                target_id,
                request,
            )
        }
    }

    impl<TO, S: MessageSender<TO>, R: MessageReceiver<ModeReply>> ModeReplyReceiver
        for MessageSenderAndReceiver<TO, ModeReply, S, R>
    {
        fn try_recv_mode_reply(
            &self,
        ) -> Result<Option<GenericMessage<ModeReply>>, GenericTargetedMessagingError> {
            self.message_receiver.try_recv_message()
        }
    }
    impl<TO, S: MessageSender<TO>, R: MessageReceiver<ModeRequest>> ModeRequestReceiver
        for MessageSenderAndReceiver<TO, ModeRequest, S, R>
    {
        fn try_recv_mode_request(
            &self,
        ) -> Result<Option<GenericMessage<ModeRequest>>, GenericTargetedMessagingError> {
            self.message_receiver.try_recv_message()
        }
    }

    impl<FROM, S: MessageSender<ModeRequest>, R: MessageReceiver<FROM>> ModeRequestSender
        for MessageSenderAndReceiver<ModeRequest, FROM, S, R>
    {
        fn local_channel_id(&self) -> ChannelId {
            self.local_channel_id_generic()
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ChannelId,
            request: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.message_sender_map.send_mode_request(
                request_id,
                self.local_channel_id(),
                target_id,
                request,
            )
        }
    }

    impl<
            REPLY,
            S0: MessageSender<ModeRequest>,
            R0: MessageReceiver<REPLY>,
            S1: MessageSender<REPLY>,
            R1: MessageReceiver<ModeRequest>,
        > RequestAndReplySenderAndReceiver<ModeRequest, REPLY, S0, R0, S1, R1>
    {
        pub fn add_request_target(&mut self, target_id: ChannelId, request_sender: S0) {
            self.request_sender_map
                .add_message_target(target_id, request_sender)
        }
    }

    impl<
            REQUEST,
            S0: MessageSender<REQUEST>,
            R0: MessageReceiver<ModeReply>,
            S1: MessageSender<ModeReply>,
            R1: MessageReceiver<REQUEST>,
        > RequestAndReplySenderAndReceiver<REQUEST, ModeReply, S0, R0, S1, R1>
    {
        pub fn add_reply_target(&mut self, target_id: ChannelId, reply_sender: S1) {
            self.reply_sender_map
                .add_message_target(target_id, reply_sender)
        }
    }

    impl<
            REPLY,
            S0: MessageSender<ModeRequest>,
            R0: MessageReceiver<REPLY>,
            S1: MessageSender<REPLY>,
            R1: MessageReceiver<ModeRequest>,
        > ModeRequestSender
        for RequestAndReplySenderAndReceiver<ModeRequest, REPLY, S0, R0, S1, R1>
    {
        fn local_channel_id(&self) -> ChannelId {
            self.local_channel_id_generic()
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ChannelId,
            request: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.request_sender_map.send_mode_request(
                request_id,
                self.local_channel_id(),
                target_id,
                request,
            )
        }
    }

    impl<
            REQUEST,
            S0: MessageSender<REQUEST>,
            R0: MessageReceiver<ModeReply>,
            S1: MessageSender<ModeReply>,
            R1: MessageReceiver<REQUEST>,
        > ModeReplySender for RequestAndReplySenderAndReceiver<REQUEST, ModeReply, S0, R0, S1, R1>
    {
        fn local_channel_id(&self) -> ChannelId {
            self.local_channel_id_generic()
        }

        fn send_mode_reply(
            &self,
            request_id: RequestId,
            target_id: ChannelId,
            request: ModeReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.reply_sender_map.send_mode_reply(
                request_id,
                self.local_channel_id(),
                target_id,
                request,
            )
        }
    }

    impl<
            REQUEST,
            S0: MessageSender<REQUEST>,
            R0: MessageReceiver<ModeReply>,
            S1: MessageSender<ModeReply>,
            R1: MessageReceiver<REQUEST>,
        > ModeReplyReceiver
        for RequestAndReplySenderAndReceiver<REQUEST, ModeReply, S0, R0, S1, R1>
    {
        fn try_recv_mode_reply(
            &self,
        ) -> Result<Option<GenericMessage<ModeReply>>, GenericTargetedMessagingError> {
            self.reply_receiver.try_recv_message()
        }
    }

    impl<
            REPLY,
            S0: MessageSender<ModeRequest>,
            R0: MessageReceiver<REPLY>,
            S1: MessageSender<REPLY>,
            R1: MessageReceiver<ModeRequest>,
        > ModeRequestReceiver
        for RequestAndReplySenderAndReceiver<ModeRequest, REPLY, S0, R0, S1, R1>
    {
        fn try_recv_mode_request(
            &self,
        ) -> Result<Option<GenericMessage<ModeRequest>>, GenericTargetedMessagingError> {
            self.request_receiver.try_recv_message()
        }
    }

    /// Helper type definition for a mode handler which can handle mode requests.
    pub type ModeRequestHandlerInterface<S, R> =
        MessageSenderAndReceiver<ModeReply, ModeRequest, S, R>;

    impl<S: MessageSender<ModeReply>, R: MessageReceiver<ModeRequest>>
        ModeRequestHandlerInterface<S, R>
    {
        pub fn try_recv_mode_request(
            &self,
        ) -> Result<Option<GenericMessage<ModeRequest>>, GenericTargetedMessagingError> {
            self.try_recv_message()
        }

        pub fn send_mode_reply(
            &self,
            request_id: RequestId,
            target_id: ChannelId,
            reply: ModeReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_id, reply)
        }
    }

    /// Helper type defintion for a mode handler object which can send mode requests.
    pub type ModeRequestorInterface<S, R> = MessageSenderAndReceiver<ModeRequest, ModeReply, S, R>;

    impl<S: MessageSender<ModeRequest>, R: MessageReceiver<ModeReply>> ModeRequestorInterface<S, R> {
        pub fn try_recv_mode_reply(
            &self,
        ) -> Result<Option<GenericMessage<ModeReply>>, GenericTargetedMessagingError> {
            self.try_recv_message()
        }

        pub fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ChannelId,
            reply: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_id, reply)
        }
    }

    /// Helper type defintion for a mode handler object which can both send mode requests and
    /// process mode requests.
    pub type ModeInterface<S0, R0, S1, R1> =
        RequestAndReplySenderAndReceiver<ModeRequest, ModeReply, S0, R0, S1, R1>;
}

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub mod std_mod {
    use std::sync::mpsc;

    use super::*;

    pub type ModeRequestHandlerMpsc = ModeRequestHandlerInterface<
        mpsc::Sender<GenericMessage<ModeReply>>,
        mpsc::Receiver<GenericMessage<ModeRequest>>,
    >;
    pub type ModeRequestHandlerMpscBounded = ModeRequestHandlerInterface<
        mpsc::SyncSender<GenericMessage<ModeReply>>,
        mpsc::Receiver<GenericMessage<ModeRequest>>,
    >;

    pub type ModeRequestorMpsc = ModeRequestorInterface<
        mpsc::Sender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
    >;
    pub type ModeRequestorBoundedMpsc = ModeRequestorInterface<
        mpsc::SyncSender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
    >;

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
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use crate::{
        mode::{ModeAndSubmode, ModeReply, ModeReplySender, ModeRequestSender},
        request::GenericMessage,
    };

    use super::{ModeRequest, ModeRequestorAndHandlerMpsc, ModeRequestorMpsc};

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
        //mode_connector.add_reply_target(TEST_CHANNEL_ID_2, reply_sender_to_channel_2);

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
}
