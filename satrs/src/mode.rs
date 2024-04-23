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
    queue::GenericTargetedMessagingError,
    request::{GenericMessage, MessageMetadata, MessageReceiver, MessageReceiverWithId, RequestId},
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
    SetMode(ModeAndSubmode),
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
    ) -> Result<(), GenericTargetedMessagingError>;
}

pub trait ModeRequestReceiver {
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<GenericMessage<ModeRequest>>, GenericTargetedMessagingError>;
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

#[derive(Debug, Clone)]
pub enum ModeError {
    Messaging(GenericTargetedMessagingError),
}

impl From<GenericTargetedMessagingError> for ModeError {
    fn from(value: GenericTargetedMessagingError) -> Self {
        Self::Messaging(value)
    }
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
            ModeRequest::SetMode(mode_and_submode) => {
                self.start_transition(request.requestor_info, mode_and_submode)
            }
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

pub trait ModeReplySender {
    fn local_channel_id(&self) -> ComponentId;

    /// The requestor is assumed to be the target of the reply.
    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError>;
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use crate::request::{
        MessageSender, MessageSenderAndReceiver, MessageSenderMap, RequestAndReplySenderAndReceiver,
    };

    use super::*;

    impl<S: MessageSender<ModeReply>> MessageSenderMap<ModeReply, S> {
        pub fn send_mode_reply(
            &self,
            requestor_info: MessageMetadata,
            target_id: ComponentId,
            request: ModeReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(requestor_info, target_id, request)
        }

        pub fn add_reply_target(&mut self, target_id: ComponentId, request_sender: S) {
            self.add_message_target(target_id, request_sender)
        }
    }

    impl<FROM, S: MessageSender<ModeReply>, R: MessageReceiver<FROM>> ModeReplySender
        for MessageSenderAndReceiver<ModeReply, FROM, S, R>
    {
        fn local_channel_id(&self) -> ComponentId {
            self.local_channel_id_generic()
        }

        fn send_mode_reply(
            &self,
            requestor_info: MessageMetadata,
            request: ModeReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.message_sender_map.send_mode_reply(
                MessageMetadata::new(requestor_info.request_id(), self.local_channel_id()),
                requestor_info.sender_id(),
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

    impl<
            REQUEST,
            S0: MessageSender<REQUEST>,
            R0: MessageReceiver<ModeReply>,
            S1: MessageSender<ModeReply>,
            R1: MessageReceiver<REQUEST>,
        > RequestAndReplySenderAndReceiver<REQUEST, ModeReply, S0, R0, S1, R1>
    {
        pub fn add_reply_target(&mut self, target_id: ComponentId, reply_sender: S1) {
            self.reply_sender_map
                .add_message_target(target_id, reply_sender)
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
        fn local_channel_id(&self) -> ComponentId {
            self.local_channel_id_generic()
        }

        fn send_mode_reply(
            &self,
            requestor_info: MessageMetadata,
            request: ModeReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.reply_sender_map.send_mode_reply(
                MessageMetadata::new(requestor_info.request_id(), self.local_channel_id()),
                requestor_info.sender_id(),
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
            requestor_info: MessageMetadata,
            reply: ModeReply,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(
                requestor_info.request_id(),
                requestor_info.sender_id(),
                reply,
            )
        }
    }

    /// Helper type defintion for a mode handler object which can send mode requests and receive
    /// mode replies.
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
            target_id: ComponentId,
            reply: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_id, reply)
        }
    }

    /// Helper type defintion for a mode handler object which can both send mode requests and
    /// process mode requests.
    pub type ModeInterface<S0, R0, S1, R1> =
        RequestAndReplySenderAndReceiver<ModeRequest, ModeReply, S0, R0, S1, R1>;

    impl<S: MessageSender<ModeRequest>> MessageSenderMap<ModeRequest, S> {
        pub fn send_mode_request(
            &self,
            requestor_info: MessageMetadata,
            target_id: ComponentId,
            request: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(requestor_info, target_id, request)
        }

        pub fn add_request_target(&mut self, target_id: ComponentId, request_sender: S) {
            self.add_message_target(target_id, request_sender)
        }
    }

    /*
    impl<S: MessageSender<ModeRequest>> ModeRequestSender for MessageSenderMapWithId<ModeRequest, S> {
        fn local_channel_id(&self) -> ComponentId {
            self.local_channel_id
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            request: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_id, request)
        }
    }
    */

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
        fn local_channel_id(&self) -> ComponentId {
            self.local_channel_id_generic()
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            request: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.message_sender_map.send_mode_request(
                MessageMetadata::new(request_id, self.local_channel_id()),
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
        pub fn add_request_target(&mut self, target_id: ComponentId, request_sender: S0) {
            self.request_sender_map
                .add_message_target(target_id, request_sender)
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
        fn local_channel_id(&self) -> ComponentId {
            self.local_channel_id_generic()
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            request: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.request_sender_map.send_mode_request(
                MessageMetadata::new(request_id, self.local_channel_id()),
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
        > ModeRequestReceiver
        for RequestAndReplySenderAndReceiver<ModeRequest, REPLY, S0, R0, S1, R1>
    {
        fn try_recv_mode_request(
            &self,
        ) -> Result<Option<GenericMessage<ModeRequest>>, GenericTargetedMessagingError> {
            self.request_receiver.try_recv_message()
        }
    }
}

#[cfg(feature = "std")]
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
mod tests {}
