use alloc::vec::Vec;
use hashbrown::HashMap;
use std::sync::mpsc;

use crate::{
    mode::{Mode, ModeAndSubmode, ModeReply, ModeRequest, Submode},
    queue::GenericTargetedMessagingError,
    request::{
        MessageReceiver, MessageReceiverWithId, MessageSender, MessageSenderAndReceiver,
        MessageSenderMap, MessageSenderMapWithId, MessageWithSenderId,
        RequestAndReplySenderAndReceiver,
    },
    ChannelId,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TableEntryType {
    /// Target table containing information of the expected children modes for  given mode.
    Target,
    /// Sequence table which contains information about how to reach a target table, including
    /// the order of the sequences.
    Sequence,
}

pub struct ModeTableEntry {
    /// Name of respective table entry.
    pub name: &'static str,
    /// Target channel ID.
    pub channel_id: ChannelId,
    pub mode_submode: ModeAndSubmode,
    pub allowed_submode_mask: Option<Submode>,
    pub check_success: bool,
}

pub struct ModeTableMapValue {
    /// Name for a given mode table entry.
    pub name: &'static str,
    pub entries: Vec<ModeTableEntry>,
}

pub type ModeTable = HashMap<Mode, ModeTableMapValue>;

pub trait ModeRequestSender {
    fn local_channel_id(&self) -> ChannelId;
    fn send_mode_request(
        &self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError>;
}

pub trait ModeReplySender {
    fn local_channel_id(&self) -> ChannelId;

    fn send_mode_reply(
        &self,
        target_id: ChannelId,
        reply: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError>;
}

pub trait ModeRequestReceiver {
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeRequest>>, GenericTargetedMessagingError>;
}

pub trait ModeReplyReceiver {
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeReply>>, GenericTargetedMessagingError>;
}

impl<S: MessageSender<ModeRequest>> MessageSenderMap<ModeRequest, S> {
    pub fn send_mode_request(
        &self,
        local_id: ChannelId,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.send_message(local_id, target_id, request)
    }

    pub fn add_request_target(&mut self, target_id: ChannelId, request_sender: S) {
        self.add_message_target(target_id, request_sender)
    }
}

impl<S: MessageSender<ModeReply>> MessageSenderMap<ModeReply, S> {
    pub fn send_mode_reply(
        &self,
        local_id: ChannelId,
        target_id: ChannelId,
        request: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.send_message(local_id, target_id, request)
    }

    pub fn add_reply_target(&mut self, target_id: ChannelId, request_sender: S) {
        self.add_message_target(target_id, request_sender)
    }
}

impl<S: MessageSender<ModeReply>> ModeReplySender for MessageSenderMapWithId<ModeReply, S> {
    fn send_mode_reply(
        &self,
        target_channel_id: ChannelId,
        reply: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.send_message(target_channel_id, reply)
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
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.send_message(target_id, request)
    }
}

impl<R: MessageReceiver<ModeReply>> ModeReplyReceiver for MessageReceiverWithId<ModeReply, R> {
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeReply>>, GenericTargetedMessagingError> {
        self.try_recv_message()
    }
}

impl<R: MessageReceiver<ModeRequest>> ModeRequestReceiver
    for MessageReceiverWithId<ModeRequest, R>
{
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeRequest>>, GenericTargetedMessagingError> {
        self.try_recv_message()
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
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.message_sender_map
            .send_mode_request(self.local_channel_id(), target_id, request)
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
        target_id: ChannelId,
        request: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.message_sender_map
            .send_mode_reply(self.local_channel_id(), target_id, request)
    }
}

impl<TO, S: MessageSender<TO>, R: MessageReceiver<ModeReply>> ModeReplyReceiver
    for MessageSenderAndReceiver<TO, ModeReply, S, R>
{
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeReply>>, GenericTargetedMessagingError> {
        self.message_receiver
            .try_recv_message(self.local_channel_id_generic())
    }
}
impl<TO, S: MessageSender<TO>, R: MessageReceiver<ModeRequest>> ModeRequestReceiver
    for MessageSenderAndReceiver<TO, ModeRequest, S, R>
{
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeRequest>>, GenericTargetedMessagingError> {
        self.message_receiver
            .try_recv_message(self.local_channel_id_generic())
    }
}

pub type ModeRequestHandlerConnector<S, R> = MessageSenderAndReceiver<ModeReply, ModeRequest, S, R>;
pub type MpscModeRequestHandlerConnector = ModeRequestHandlerConnector<
    mpsc::Sender<MessageWithSenderId<ModeReply>>,
    mpsc::Receiver<MessageWithSenderId<ModeRequest>>,
>;
pub type MpscBoundedModeRequestHandlerConnector = ModeRequestHandlerConnector<
    mpsc::SyncSender<MessageWithSenderId<ModeReply>>,
    mpsc::Receiver<MessageWithSenderId<ModeRequest>>,
>;

pub type ModeRequestorConnector<S, R> = MessageSenderAndReceiver<ModeRequest, ModeReply, S, R>;
pub type MpscModeRequestorConnector = ModeRequestorConnector<
    mpsc::Sender<MessageWithSenderId<ModeRequest>>,
    mpsc::Receiver<MessageWithSenderId<ModeReply>>,
>;
pub type MpscBoundedModeRequestorConnector = ModeRequestorConnector<
    mpsc::SyncSender<MessageWithSenderId<ModeRequest>>,
    mpsc::Receiver<MessageWithSenderId<ModeReply>>,
>;

pub type ModeConnector<S0, R0, S1, R1> =
    RequestAndReplySenderAndReceiver<ModeRequest, ModeReply, S0, R0, S1, R1>;
pub type MpscModeConnector = ModeConnector<
    mpsc::Sender<MessageWithSenderId<ModeRequest>>,
    mpsc::Receiver<MessageWithSenderId<ModeReply>>,
    mpsc::Sender<MessageWithSenderId<ModeReply>>,
    mpsc::Receiver<MessageWithSenderId<ModeRequest>>,
>;
pub type MpscBoundedModeConnector = ModeConnector<
    mpsc::SyncSender<MessageWithSenderId<ModeRequest>>,
    mpsc::Receiver<MessageWithSenderId<ModeReply>>,
    mpsc::SyncSender<MessageWithSenderId<ModeReply>>,
    mpsc::Receiver<MessageWithSenderId<ModeRequest>>,
>;

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
    > ModeRequestSender for RequestAndReplySenderAndReceiver<ModeRequest, REPLY, S0, R0, S1, R1>
{
    fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id_generic()
    }

    fn send_mode_request(
        &self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.request_sender_map
            .send_mode_request(self.local_channel_id(), target_id, request)
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
        target_id: ChannelId,
        request: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.reply_sender_map
            .send_mode_reply(self.local_channel_id(), target_id, request)
    }
}

impl<
        REQUEST,
        S0: MessageSender<REQUEST>,
        R0: MessageReceiver<ModeReply>,
        S1: MessageSender<ModeReply>,
        R1: MessageReceiver<REQUEST>,
    > ModeReplyReceiver for RequestAndReplySenderAndReceiver<REQUEST, ModeReply, S0, R0, S1, R1>
{
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeReply>>, GenericTargetedMessagingError> {
        self.reply_receiver
            .try_recv_message(self.local_channel_id_generic())
    }
}

impl<
        REPLY,
        S0: MessageSender<ModeRequest>,
        R0: MessageReceiver<REPLY>,
        S1: MessageSender<REPLY>,
        R1: MessageReceiver<ModeRequest>,
    > ModeRequestReceiver for RequestAndReplySenderAndReceiver<ModeRequest, REPLY, S0, R0, S1, R1>
{
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeRequest>>, GenericTargetedMessagingError> {
        self.request_receiver
            .try_recv_message(self.local_channel_id_generic())
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
    fn start_transition(&mut self, mode_and_submode: ModeAndSubmode) -> Result<(), ModeError>;

    fn announce_mode(&self, recursive: bool);
    fn handle_mode_reached(&mut self) -> Result<(), GenericTargetedMessagingError>;
}

#[cfg(test)]
mod tests {}
