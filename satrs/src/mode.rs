use core::mem::size_of;
use satrs_shared::res_code::ResultU16;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use spacepackets::ByteConversionError;

#[cfg(feature = "alloc")]
pub use alloc_mod::*;

#[cfg(feature = "std")]
pub use std_mod::*;

use crate::{
    queue::{GenericReceiveError, GenericSendError},
    request::{
        GenericMessage, MessageMetadata, MessageReceiverProvider, MessageReceiverWithId, RequestId,
    },
    ComponentId,
};

pub type Mode = u32;
pub type Submode = u16;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModeAndSubmode {
    mode: Mode,
    submode: Submode,
}

pub const INVALID_MODE_VAL: Mode = Mode::MAX;
pub const UNKNOWN_MODE_VAL: Mode = Mode::MAX - 1;
pub const INVALID_MODE: ModeAndSubmode = ModeAndSubmode::new(INVALID_MODE_VAL, 0);
pub const UNKNOWN_MODE: ModeAndSubmode = ModeAndSubmode::new(UNKNOWN_MODE_VAL, 0);

impl ModeAndSubmode {
    pub const RAW_LEN: usize = size_of::<Mode>() + size_of::<Submode>();

    pub const fn new_mode_only(mode: Mode) -> Self {
        Self { mode, submode: 0 }
    }

    pub const fn new(mode: Mode, submode: Submode) -> Self {
        Self { mode, submode }
    }

    pub fn from_be_bytes(buf: &[u8]) -> Result<Self, ByteConversionError> {
        if buf.len() < 6 {
            return Err(ByteConversionError::FromSliceTooSmall {
                expected: Self::RAW_LEN,
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

    pub fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError> {
        if buf.len() < Self::RAW_LEN {
            return Err(ByteConversionError::ToSliceTooSmall {
                expected: Self::RAW_LEN,
                found: buf.len(),
            });
        }
        buf[0..size_of::<Mode>()].copy_from_slice(&self.mode.to_be_bytes());
        buf[size_of::<Mode>()..Self::RAW_LEN].copy_from_slice(&self.submode.to_be_bytes());
        Ok(Self::RAW_LEN)
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
    pub address: ComponentId,
    pub mode_submode: ModeAndSubmode,
}

impl TargetedModeCommand {
    pub const fn new(address: ComponentId, mode_submode: ModeAndSubmode) -> Self {
        Self {
            address,
            mode_submode,
        }
    }

    pub fn address(&self) -> ComponentId {
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
    /// Mode information. Can be used to notify other components of changed modes.
    ModeInfo(ModeAndSubmode),
    SetMode {
        mode_and_submode: ModeAndSubmode,
        forced: bool,
    },
    ReadMode,
    AnnounceMode,
    AnnounceModeRecursive,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TargetedModeRequest {
    target_id: ComponentId,
    mode_request: ModeRequest,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ModeReply {
    /// Mode information. Can be used to notify other components of changed modes.
    ModeInfo(ModeAndSubmode),
    /// Reply to a mode request to confirm the commanded mode was reached.
    ModeReply(ModeAndSubmode),
    // Can not reach the commanded mode. Contains a reason as a [ResultU16].
    CantReachMode(ResultU16),
    /// We are in the wrong mode for unknown reasons. Contains the expected and reached mode.
    WrongMode {
        expected: ModeAndSubmode,
        reached: ModeAndSubmode,
    },
}

pub type GenericModeReply = GenericMessage<ModeReply>;

pub trait ModeRequestSender {
    fn local_channel_id(&self) -> ComponentId;
    fn send_mode_request(
        &self,
        request_id: RequestId,
        target_id: ComponentId,
        request: ModeRequest,
    ) -> Result<(), GenericSendError>;
}

pub trait ModeRequestReceiver {
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<GenericMessage<ModeRequest>>, GenericReceiveError>;
}

impl<R: MessageReceiverProvider<ModeRequest>> ModeRequestReceiver
    for MessageReceiverWithId<ModeRequest, R>
{
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<GenericMessage<ModeRequest>>, GenericReceiveError> {
        self.try_recv_message()
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ModeError {
    #[error("Messaging send error: {0}")]
    Send(#[from] GenericSendError),
    #[error("Messaging receive error: {0}")]
    Receive(#[from] GenericReceiveError),
    #[error("busy with other mode request")]
    Busy,
}

pub trait ModeProvider {
    fn mode_and_submode(&self) -> ModeAndSubmode;

    fn mode(&self) -> Mode {
        self.mode_and_submode().mode()
    }

    fn submode(&self) -> Submode {
        self.mode_and_submode().submode()
    }
}

pub trait ModeRequestHandler: ModeProvider {
    type Error;

    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
    ) -> Result<(), Self::Error>;

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool);

    fn handle_mode_reached(
        &mut self,
        requestor_info: Option<MessageMetadata>,
    ) -> Result<(), Self::Error>;

    fn handle_mode_info(
        &mut self,
        requestor_info: MessageMetadata,
        info: ModeAndSubmode,
    ) -> Result<(), Self::Error>;

    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), Self::Error>;

    fn handle_mode_request(
        &mut self,
        request: GenericMessage<ModeRequest>,
    ) -> Result<(), Self::Error> {
        match request.message {
            ModeRequest::SetMode {
                mode_and_submode,
                forced,
            } => self.start_transition(request.requestor_info, mode_and_submode, forced),
            ModeRequest::ReadMode => self.send_mode_reply(
                request.requestor_info,
                ModeReply::ModeReply(self.mode_and_submode()),
            ),
            ModeRequest::AnnounceMode => {
                self.announce_mode(Some(request.requestor_info), false);
                Ok(())
            }
            ModeRequest::AnnounceModeRecursive => {
                self.announce_mode(Some(request.requestor_info), true);
                Ok(())
            }
            ModeRequest::ModeInfo(info) => self.handle_mode_info(request.requestor_info, info),
        }
    }
}

pub trait ModeReplyReceiver {
    fn try_recv_mode_reply(&self)
        -> Result<Option<GenericMessage<ModeReply>>, GenericReceiveError>;
}

impl<R: MessageReceiverProvider<ModeReply>> ModeReplyReceiver
    for MessageReceiverWithId<ModeReply, R>
{
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<GenericMessage<ModeReply>>, GenericReceiveError> {
        self.try_recv_message()
    }
}

pub trait ModeReplySender {
    fn local_channel_id(&self) -> ComponentId;

    /// The requestor is assumed to be the target of the reply.
    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), GenericSendError>;
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use crate::{
        queue::{GenericReceiveError, GenericSendError},
        request::{
            MessageSenderAndReceiver, MessageSenderMap, MessageSenderProvider,
            MessageSenderStoreProvider, RequestAndReplySenderAndReceiver,
        },
    };

    use super::*;

    impl<S: MessageSenderProvider<ModeReply>> MessageSenderMap<ModeReply, S> {
        pub fn send_mode_reply(
            &self,
            requestor_info: MessageMetadata,
            target_id: ComponentId,
            request: ModeReply,
        ) -> Result<(), GenericSendError> {
            self.send_message(requestor_info, target_id, request)
        }

        pub fn add_reply_target(&mut self, target_id: ComponentId, request_sender: S) {
            self.add_message_target(target_id, request_sender)
        }
    }

    impl<
            From,
            Sender: MessageSenderProvider<ModeReply>,
            Receiver: MessageReceiverProvider<From>,
            SenderStore: MessageSenderStoreProvider<ModeReply, Sender>,
        > ModeReplySender
        for MessageSenderAndReceiver<ModeReply, From, Sender, Receiver, SenderStore>
    {
        fn local_channel_id(&self) -> ComponentId {
            self.local_channel_id_generic()
        }

        fn send_mode_reply(
            &self,
            requestor_info: MessageMetadata,
            request: ModeReply,
        ) -> Result<(), GenericSendError> {
            self.message_sender_store.send_message(
                MessageMetadata::new(requestor_info.request_id(), self.local_channel_id()),
                requestor_info.sender_id(),
                request,
            )
        }
    }

    impl<
            To,
            Sender: MessageSenderProvider<To>,
            Receiver: MessageReceiverProvider<ModeReply>,
            SenderStore: MessageSenderStoreProvider<To, Sender>,
        > ModeReplyReceiver
        for MessageSenderAndReceiver<To, ModeReply, Sender, Receiver, SenderStore>
    {
        fn try_recv_mode_reply(
            &self,
        ) -> Result<Option<GenericMessage<ModeReply>>, GenericReceiveError> {
            self.message_receiver.try_recv_message()
        }
    }

    impl<
            Request,
            ReqSender: MessageSenderProvider<Request>,
            ReqReceiver: MessageReceiverProvider<Request>,
            ReqSenderStore: MessageSenderStoreProvider<Request, ReqSender>,
            Reply,
            ReplySender: MessageSenderProvider<Reply>,
            ReplyReceiver: MessageReceiverProvider<Reply>,
            ReplySenderStore: MessageSenderStoreProvider<Reply, ReplySender>,
        >
        RequestAndReplySenderAndReceiver<
            Request,
            ReqSender,
            ReqReceiver,
            ReqSenderStore,
            Reply,
            ReplySender,
            ReplyReceiver,
            ReplySenderStore,
        >
    {
        pub fn add_reply_target(&mut self, target_id: ComponentId, reply_sender: ReplySender) {
            self.reply_sender_store
                .add_message_target(target_id, reply_sender)
        }
    }

    impl<
            Request,
            ReqSender: MessageSenderProvider<Request>,
            ReqReceiver: MessageReceiverProvider<Request>,
            ReqSenderStore: MessageSenderStoreProvider<Request, ReqSender>,
            ReplySender: MessageSenderProvider<ModeReply>,
            ReplyReceiver: MessageReceiverProvider<ModeReply>,
            ReplySenderStore: MessageSenderStoreProvider<ModeReply, ReplySender>,
        > ModeReplySender
        for RequestAndReplySenderAndReceiver<
            Request,
            ReqSender,
            ReqReceiver,
            ReqSenderStore,
            ModeReply,
            ReplySender,
            ReplyReceiver,
            ReplySenderStore,
        >
    {
        fn local_channel_id(&self) -> ComponentId {
            self.local_channel_id_generic()
        }

        fn send_mode_reply(
            &self,
            requestor_info: MessageMetadata,
            reply: ModeReply,
        ) -> Result<(), GenericSendError> {
            self.reply_sender_store.send_message(
                MessageMetadata::new(requestor_info.request_id(), self.local_channel_id()),
                requestor_info.sender_id(),
                reply,
            )
        }
    }

    impl<
            Request,
            ReqSender: MessageSenderProvider<Request>,
            ReqReceiver: MessageReceiverProvider<Request>,
            ReqSenderStore: MessageSenderStoreProvider<Request, ReqSender>,
            ReplySender: MessageSenderProvider<ModeReply>,
            ReplyReceiver: MessageReceiverProvider<ModeReply>,
            ReplySenderStore: MessageSenderStoreProvider<ModeReply, ReplySender>,
        > ModeReplyReceiver
        for RequestAndReplySenderAndReceiver<
            Request,
            ReqSender,
            ReqReceiver,
            ReqSenderStore,
            ModeReply,
            ReplySender,
            ReplyReceiver,
            ReplySenderStore,
        >
    {
        fn try_recv_mode_reply(
            &self,
        ) -> Result<Option<GenericMessage<ModeReply>>, GenericReceiveError> {
            self.reply_receiver.try_recv_message()
        }
    }

    /// Helper type definition for a mode handler which can handle mode requests.
    pub type ModeRequestHandlerInterface<Sender, Receiver, ReplySenderStore> =
        MessageSenderAndReceiver<ModeReply, ModeRequest, Sender, Receiver, ReplySenderStore>;

    impl<
            Sender: MessageSenderProvider<ModeReply>,
            Receiver: MessageReceiverProvider<ModeRequest>,
            ReplySenderStore: MessageSenderStoreProvider<ModeReply, Sender>,
        > ModeRequestHandlerInterface<Sender, Receiver, ReplySenderStore>
    {
        pub fn try_recv_mode_request(
            &self,
        ) -> Result<Option<GenericMessage<ModeRequest>>, GenericReceiveError> {
            self.try_recv_message()
        }

        pub fn send_mode_reply(
            &self,
            requestor_info: MessageMetadata,
            reply: ModeReply,
        ) -> Result<(), GenericSendError> {
            self.send_message(
                requestor_info.request_id(),
                requestor_info.sender_id(),
                reply,
            )
        }
    }

    /// Helper type defintion for a mode handler object which can send mode requests and receive
    /// mode replies.
    pub type ModeRequestorInterface<Sender, Receiver, RequestSenderStore> =
        MessageSenderAndReceiver<ModeRequest, ModeReply, Sender, Receiver, RequestSenderStore>;

    impl<
            Sender: MessageSenderProvider<ModeRequest>,
            Receiver: MessageReceiverProvider<ModeReply>,
            RequestSenderStore: MessageSenderStoreProvider<ModeRequest, Sender>,
        > ModeRequestorInterface<Sender, Receiver, RequestSenderStore>
    {
        pub fn try_recv_mode_reply(
            &self,
        ) -> Result<Option<GenericMessage<ModeReply>>, GenericReceiveError> {
            self.try_recv_message()
        }

        pub fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            reply: ModeRequest,
        ) -> Result<(), GenericSendError> {
            self.send_message(request_id, target_id, reply)
        }
    }

    /// Helper type defintion for a mode handler object which can both send mode requests and
    /// process mode requests.
    pub type ModeInterface<
        ReqSender,
        ReqReceiver,
        ReqSenderStore,
        ReplySender,
        ReplyReceiver,
        ReplySenderStore,
    > = RequestAndReplySenderAndReceiver<
        ModeRequest,
        ReqSender,
        ReqReceiver,
        ReqSenderStore,
        ModeReply,
        ReplySender,
        ReplyReceiver,
        ReplySenderStore,
    >;

    impl<S: MessageSenderProvider<ModeRequest>> MessageSenderMap<ModeRequest, S> {
        pub fn send_mode_request(
            &self,
            requestor_info: MessageMetadata,
            target_id: ComponentId,
            request: ModeRequest,
        ) -> Result<(), GenericSendError> {
            self.send_message(requestor_info, target_id, request)
        }

        pub fn add_request_target(&mut self, target_id: ComponentId, request_sender: S) {
            self.add_message_target(target_id, request_sender)
        }
    }

    impl<
            To,
            Sender: MessageSenderProvider<To>,
            Receiver: MessageReceiverProvider<ModeRequest>,
            SenderStore: MessageSenderStoreProvider<To, Sender>,
        > ModeRequestReceiver
        for MessageSenderAndReceiver<To, ModeRequest, Sender, Receiver, SenderStore>
    {
        fn try_recv_mode_request(
            &self,
        ) -> Result<Option<GenericMessage<ModeRequest>>, GenericReceiveError> {
            self.message_receiver.try_recv_message()
        }
    }

    impl<
            From,
            Sender: MessageSenderProvider<ModeRequest>,
            Receiver: MessageReceiverProvider<From>,
            SenderStore: MessageSenderStoreProvider<ModeRequest, Sender>,
        > ModeRequestSender
        for MessageSenderAndReceiver<ModeRequest, From, Sender, Receiver, SenderStore>
    {
        fn local_channel_id(&self) -> ComponentId {
            self.local_channel_id_generic()
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            request: ModeRequest,
        ) -> Result<(), GenericSendError> {
            self.message_sender_store.send_message(
                MessageMetadata::new(request_id, self.local_channel_id()),
                target_id,
                request,
            )
        }
    }

    impl<
            ReqSender: MessageSenderProvider<ModeRequest>,
            ReqReceiver: MessageReceiverProvider<ModeRequest>,
            ReqSenderStore: MessageSenderStoreProvider<ModeRequest, ReqSender>,
            Reply,
            ReplySender: MessageSenderProvider<Reply>,
            ReplyReceiver: MessageReceiverProvider<Reply>,
            ReplySenderStore: MessageSenderStoreProvider<Reply, ReplySender>,
        >
        RequestAndReplySenderAndReceiver<
            ModeRequest,
            ReqSender,
            ReqReceiver,
            ReqSenderStore,
            Reply,
            ReplySender,
            ReplyReceiver,
            ReplySenderStore,
        >
    {
        pub fn add_request_target(&mut self, target_id: ComponentId, request_sender: ReqSender) {
            self.request_sender_store
                .add_message_target(target_id, request_sender)
        }
    }

    impl<
            ReqSender: MessageSenderProvider<ModeRequest>,
            ReqReceiver: MessageReceiverProvider<ModeRequest>,
            ReqSenderStore: MessageSenderStoreProvider<ModeRequest, ReqSender>,
            Reply,
            ReplySender: MessageSenderProvider<Reply>,
            ReplyReceiver: MessageReceiverProvider<Reply>,
            ReplySenderStore: MessageSenderStoreProvider<Reply, ReplySender>,
        > ModeRequestSender
        for RequestAndReplySenderAndReceiver<
            ModeRequest,
            ReqSender,
            ReqReceiver,
            ReqSenderStore,
            Reply,
            ReplySender,
            ReplyReceiver,
            ReplySenderStore,
        >
    {
        fn local_channel_id(&self) -> ComponentId {
            self.local_channel_id_generic()
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            request: ModeRequest,
        ) -> Result<(), GenericSendError> {
            self.request_sender_store.send_message(
                MessageMetadata::new(request_id, self.local_channel_id()),
                target_id,
                request,
            )
        }
    }

    impl<
            ReqSender: MessageSenderProvider<ModeRequest>,
            ReqReceiver: MessageReceiverProvider<ModeRequest>,
            ReqSenderStore: MessageSenderStoreProvider<ModeRequest, ReqSender>,
            Reply,
            ReplySender: MessageSenderProvider<Reply>,
            ReplyReceiver: MessageReceiverProvider<Reply>,
            ReplySenderStore: MessageSenderStoreProvider<Reply, ReplySender>,
        > ModeRequestReceiver
        for RequestAndReplySenderAndReceiver<
            ModeRequest,
            ReqSender,
            ReqReceiver,
            ReqSenderStore,
            Reply,
            ReplySender,
            ReplyReceiver,
            ReplySenderStore,
        >
    {
        fn try_recv_mode_request(
            &self,
        ) -> Result<Option<GenericMessage<ModeRequest>>, GenericReceiveError> {
            self.request_receiver.try_recv_message()
        }
    }
}

#[cfg(feature = "std")]
pub mod std_mod {
    use std::sync::mpsc;

    use crate::request::{MessageSenderList, OneMessageSender};

    use super::*;

    pub type ModeRequestHandlerMpsc = ModeRequestHandlerInterface<
        mpsc::Sender<GenericMessage<ModeReply>>,
        mpsc::Receiver<GenericMessage<ModeRequest>>,
        MessageSenderList<ModeReply, mpsc::Sender<GenericMessage<ModeReply>>>,
    >;
    pub type ModeRequestHandlerMpscBounded = ModeRequestHandlerInterface<
        mpsc::SyncSender<GenericMessage<ModeReply>>,
        mpsc::Receiver<GenericMessage<ModeRequest>>,
        MessageSenderList<ModeReply, mpsc::SyncSender<GenericMessage<ModeReply>>>,
    >;

    pub type ModeRequestorOneChildMpsc = ModeRequestorInterface<
        mpsc::Sender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
        OneMessageSender<ModeRequest, mpsc::Sender<GenericMessage<ModeRequest>>>,
    >;
    pub type ModeRequestorOneChildBoundedMpsc = ModeRequestorInterface<
        mpsc::SyncSender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
        OneMessageSender<ModeRequest, mpsc::SyncSender<GenericMessage<ModeRequest>>>,
    >;
    pub type ModeRequestorChildListMpsc = ModeRequestorInterface<
        mpsc::Sender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
        MessageSenderList<ModeRequest, mpsc::Sender<GenericMessage<ModeRequest>>>,
    >;
    pub type ModeRequestorChildListBoundedMpsc = ModeRequestorInterface<
        mpsc::SyncSender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
        MessageSenderList<ModeRequest, mpsc::SyncSender<GenericMessage<ModeRequest>>>,
    >;

    pub type ModeRequestorAndHandlerMpsc = ModeInterface<
        mpsc::Sender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeRequest>>,
        MessageSenderList<ModeRequest, mpsc::Sender<GenericMessage<ModeRequest>>>,
        mpsc::Sender<GenericMessage<ModeReply>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
        MessageSenderList<ModeReply, mpsc::Sender<GenericMessage<ModeReply>>>,
    >;
    pub type ModeRequestorAndHandlerMpscBounded = ModeInterface<
        mpsc::SyncSender<GenericMessage<ModeRequest>>,
        mpsc::Receiver<GenericMessage<ModeRequest>>,
        MessageSenderList<ModeRequest, mpsc::SyncSender<GenericMessage<ModeRequest>>>,
        mpsc::SyncSender<GenericMessage<ModeReply>>,
        mpsc::Receiver<GenericMessage<ModeReply>>,
        MessageSenderList<ModeReply, mpsc::SyncSender<GenericMessage<ModeReply>>>,
    >;
}

#[cfg(test)]
pub(crate) mod tests {
    use core::cell::RefCell;
    use std::collections::VecDeque;

    use crate::{request::RequestId, ComponentId};

    use super::*;

    pub struct ModeReqWrapper {
        pub request_id: RequestId,
        pub target_id: ComponentId,
        pub request: ModeRequest,
    }

    #[derive(Default)]
    pub struct ModeReqSenderMock {
        pub requests: RefCell<VecDeque<ModeReqWrapper>>,
    }

    impl ModeRequestSender for ModeReqSenderMock {
        fn local_channel_id(&self) -> crate::ComponentId {
            0
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            request: ModeRequest,
        ) -> Result<(), GenericSendError> {
            self.requests.borrow_mut().push_back(ModeReqWrapper {
                request_id,
                target_id,
                request,
            });
            Ok(())
        }
    }
}
