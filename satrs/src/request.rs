use core::{fmt, marker::PhantomData};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "std")]
use std::error::Error;

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub use alloc_mod::*;

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub use std_mod::*;

use spacepackets::{
    ecss::{tc::IsPusTelecommand, PusPacket},
    ByteConversionError, CcsdsPacket,
};

use crate::{queue::GenericTargetedMessagingError, ChannelId, TargetId};

/// Generic request ID type. Requests can be associated with an ID to have a unique identifier
/// for them. This can be useful for tasks like tracking their progress.
pub type RequestId = u32;

/// CCSDS APID type definition. Please note that the APID is a 14 bit value.
pub type Apid = u16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetIdCreationError {
    ByteConversion(ByteConversionError),
    NotEnoughAppData(usize),
}

impl From<ByteConversionError> for TargetIdCreationError {
    fn from(e: ByteConversionError) -> Self {
        Self::ByteConversion(e)
    }
}

impl fmt::Display for TargetIdCreationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::ByteConversion(e) => write!(f, "target ID creation: {}", e),
            Self::NotEnoughAppData(len) => {
                write!(f, "not enough app data to generate target ID: {}", len)
            }
        }
    }
}

#[cfg(feature = "std")]
impl Error for TargetIdCreationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let Self::ByteConversion(e) = self {
            return Some(e);
        }
        None
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct TargetAndApidId {
    pub apid: Apid,
    pub target: u32,
}

impl TargetAndApidId {
    pub fn new(apid: Apid, target: u32) -> Self {
        Self { apid, target }
    }

    pub fn apid(&self) -> Apid {
        self.apid
    }

    pub fn target(&self) -> u32 {
        self.target
    }

    pub fn raw(&self) -> TargetId {
        ((self.apid as u64) << 32) | (self.target as u64)
    }

    pub fn target_id(&self) -> TargetId {
        self.raw()
    }

    pub fn from_pus_tc(
        tc: &(impl CcsdsPacket + PusPacket + IsPusTelecommand),
    ) -> Result<Self, TargetIdCreationError> {
        if tc.user_data().len() < 4 {
            return Err(ByteConversionError::FromSliceTooSmall {
                found: tc.user_data().len(),
                expected: 8,
            }
            .into());
        }
        Ok(Self {
            apid: tc.apid(),
            target: u32::from_be_bytes(tc.user_data()[0..4].try_into().unwrap()),
        })
    }
}

impl From<u64> for TargetAndApidId {
    fn from(raw: u64) -> Self {
        Self {
            apid: (raw >> 32) as u16,
            target: raw as u32,
        }
    }
}

impl From<TargetAndApidId> for u64 {
    fn from(target_and_apid_id: TargetAndApidId) -> Self {
        target_and_apid_id.raw()
    }
}

impl fmt::Display for TargetAndApidId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}, {}", self.apid, self.target)
    }
}

/// Generic message type which is associated with a sender using a [ChannelId] and associated
/// with a request using a [RequestId].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GenericMessage<MSG> {
    pub sender_id: ChannelId,
    pub request_id: RequestId,
    pub message: MSG,
}

impl<MSG> GenericMessage<MSG> {
    pub fn new(request_id: RequestId, sender_id: ChannelId, message: MSG) -> Self {
        Self {
            request_id,
            sender_id,
            message,
        }
    }
}

/// Generic trait for objects which can send targeted messages.
pub trait MessageSender<MSG>: Send {
    fn send(&self, message: GenericMessage<MSG>) -> Result<(), GenericTargetedMessagingError>;
}

// Generic trait for objects which can receive targeted messages.
pub trait MessageReceiver<MSG> {
    fn try_recv(&self) -> Result<Option<GenericMessage<MSG>>, GenericTargetedMessagingError>;
}

pub struct MessageWithSenderIdReceiver<MSG, R: MessageReceiver<MSG>>(pub R, PhantomData<MSG>);

impl<MSG, R: MessageReceiver<MSG>> From<R> for MessageWithSenderIdReceiver<MSG, R> {
    fn from(receiver: R) -> Self {
        MessageWithSenderIdReceiver(receiver, PhantomData)
    }
}

impl<MSG, R: MessageReceiver<MSG>> MessageWithSenderIdReceiver<MSG, R> {
    pub fn try_recv_message(
        &self,
    ) -> Result<Option<GenericMessage<MSG>>, GenericTargetedMessagingError> {
        self.0.try_recv()
    }
}

impl<MSG, R: MessageReceiver<MSG>> MessageReceiverWithId<MSG, R> {
    pub fn new(
        local_channel_id: ChannelId,
        reply_receiver: MessageWithSenderIdReceiver<MSG, R>,
    ) -> Self {
        Self {
            local_channel_id,
            reply_receiver,
        }
    }

    pub fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id
    }
}

impl<MSG, R: MessageReceiver<MSG>> MessageReceiverWithId<MSG, R> {
    pub fn try_recv_message(
        &self,
    ) -> Result<Option<GenericMessage<MSG>>, GenericTargetedMessagingError> {
        self.reply_receiver.0.try_recv()
    }
}

pub struct MessageReceiverWithId<MSG, R: MessageReceiver<MSG>> {
    local_channel_id: ChannelId,
    reply_receiver: MessageWithSenderIdReceiver<MSG, R>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod alloc_mod {
    use core::marker::PhantomData;

    use crate::queue::GenericSendError;

    use super::*;
    use hashbrown::HashMap;

    pub struct MessageSenderMap<MSG, S: MessageSender<MSG>>(
        pub HashMap<ChannelId, S>,
        pub(crate) PhantomData<MSG>,
    );

    impl<MSG, S: MessageSender<MSG>> Default for MessageSenderMap<MSG, S> {
        fn default() -> Self {
            Self(Default::default(), PhantomData)
        }
    }

    impl<MSG, S: MessageSender<MSG>> MessageSenderMap<MSG, S> {
        pub fn add_message_target(&mut self, target_id: ChannelId, message_sender: S) {
            self.0.insert(target_id, message_sender);
        }

        pub fn send_message(
            &self,
            request_id: RequestId,
            local_channel_id: ChannelId,
            target_channel_id: ChannelId,
            message: MSG,
        ) -> Result<(), GenericTargetedMessagingError> {
            if self.0.contains_key(&target_channel_id) {
                self.0
                    .get(&target_channel_id)
                    .unwrap()
                    .send(GenericMessage::new(request_id, local_channel_id, message))
                    .map_err(|_| GenericSendError::RxDisconnected)?;
                return Ok(());
            }
            Err(GenericSendError::TargetDoesNotExist(target_channel_id).into())
        }
    }

    pub struct MessageSenderMapWithId<MSG, S: MessageSender<MSG>> {
        pub local_channel_id: ChannelId,
        pub message_sender_map: MessageSenderMap<MSG, S>,
    }

    impl<MSG, S: MessageSender<MSG>> MessageSenderMapWithId<MSG, S> {
        pub fn new(local_channel_id: ChannelId) -> Self {
            Self {
                local_channel_id,
                message_sender_map: Default::default(),
            }
        }

        pub fn send_message(
            &self,
            request_id: RequestId,
            target_channel_id: ChannelId,
            message: MSG,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.message_sender_map.send_message(
                request_id,
                self.local_channel_id,
                target_channel_id,
                message,
            )
        }

        pub fn add_message_target(&mut self, target_id: ChannelId, message_sender: S) {
            self.message_sender_map
                .add_message_target(target_id, message_sender)
        }
    }

    pub struct MessageSenderAndReceiver<TO, FROM, S: MessageSender<TO>, R: MessageReceiver<FROM>> {
        pub local_channel_id: ChannelId,
        pub message_sender_map: MessageSenderMap<TO, S>,
        pub message_receiver: MessageWithSenderIdReceiver<FROM, R>,
    }

    impl<TO, FROM, S: MessageSender<TO>, R: MessageReceiver<FROM>>
        MessageSenderAndReceiver<TO, FROM, S, R>
    {
        pub fn new(local_channel_id: ChannelId, message_receiver: R) -> Self {
            Self {
                local_channel_id,
                message_sender_map: Default::default(),
                message_receiver: MessageWithSenderIdReceiver::from(message_receiver),
            }
        }

        pub fn add_message_target(&mut self, target_id: ChannelId, message_sender: S) {
            self.message_sender_map
                .add_message_target(target_id, message_sender)
        }

        pub fn local_channel_id_generic(&self) -> ChannelId {
            self.local_channel_id
        }

        /// Try to send a message, which can be a reply or a request, depending on the generics.
        pub fn send_message(
            &self,
            request_id: RequestId,
            target_channel_id: ChannelId,
            message: TO,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.message_sender_map.send_message(
                request_id,
                self.local_channel_id_generic(),
                target_channel_id,
                message,
            )
        }

        /// Try to receive a message, which can be a reply or a request, depending on the generics.
        pub fn try_recv_message(
            &self,
        ) -> Result<Option<GenericMessage<FROM>>, GenericTargetedMessagingError> {
            self.message_receiver.try_recv_message()
        }
    }

    pub struct RequestAndReplySenderAndReceiver<
        REQUEST,
        REPLY,
        S0: MessageSender<REQUEST>,
        R0: MessageReceiver<REPLY>,
        S1: MessageSender<REPLY>,
        R1: MessageReceiver<REQUEST>,
    > {
        pub local_channel_id: ChannelId,
        // These 2 are a functional group.
        pub request_sender_map: MessageSenderMap<REQUEST, S0>,
        pub reply_receiver: MessageWithSenderIdReceiver<REPLY, R0>,
        // These 2 are a functional group.
        pub request_receiver: MessageWithSenderIdReceiver<REQUEST, R1>,
        pub reply_sender_map: MessageSenderMap<REPLY, S1>,
    }

    impl<
            REQUEST,
            REPLY,
            S0: MessageSender<REQUEST>,
            R0: MessageReceiver<REPLY>,
            S1: MessageSender<REPLY>,
            R1: MessageReceiver<REQUEST>,
        > RequestAndReplySenderAndReceiver<REQUEST, REPLY, S0, R0, S1, R1>
    {
        pub fn new(local_channel_id: ChannelId, request_receiver: R1, reply_receiver: R0) -> Self {
            Self {
                local_channel_id,
                request_receiver: request_receiver.into(),
                reply_receiver: reply_receiver.into(),
                request_sender_map: Default::default(),
                reply_sender_map: Default::default(),
            }
        }

        pub fn local_channel_id_generic(&self) -> ChannelId {
            self.local_channel_id
        }
    }
}

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub mod std_mod {

    use super::*;
    use std::sync::mpsc;

    use crate::queue::{GenericReceiveError, GenericSendError, GenericTargetedMessagingError};

    impl<MSG: Send> MessageSender<MSG> for mpsc::Sender<GenericMessage<MSG>> {
        fn send(&self, message: GenericMessage<MSG>) -> Result<(), GenericTargetedMessagingError> {
            self.send(message)
                .map_err(|_| GenericSendError::RxDisconnected)?;
            Ok(())
        }
    }
    impl<MSG: Send> MessageSender<MSG> for mpsc::SyncSender<GenericMessage<MSG>> {
        fn send(&self, message: GenericMessage<MSG>) -> Result<(), GenericTargetedMessagingError> {
            if let Err(e) = self.try_send(message) {
                match e {
                    mpsc::TrySendError::Full(_) => {
                        return Err(GenericSendError::QueueFull(None).into());
                    }
                    mpsc::TrySendError::Disconnected(_) => todo!(),
                }
            }
            Ok(())
        }
    }

    pub type MessageSenderMapMpsc<MSG> = MessageReceiverWithId<MSG, mpsc::Sender<MSG>>;
    pub type MessageSenderMapBoundedMpsc<MSG> = MessageReceiverWithId<MSG, mpsc::SyncSender<MSG>>;

    impl<MSG> MessageReceiver<MSG> for mpsc::Receiver<GenericMessage<MSG>> {
        fn try_recv(&self) -> Result<Option<GenericMessage<MSG>>, GenericTargetedMessagingError> {
            match self.try_recv() {
                Ok(msg) => Ok(Some(msg)),
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => Ok(None),
                    mpsc::TryRecvError::Disconnected => {
                        Err(GenericReceiveError::TxDisconnected(None).into())
                    }
                },
            }
        }
    }

    pub type MessageReceiverWithIdMpsc<MSG> = MessageReceiverWithId<MSG, mpsc::Receiver<MSG>>;
}
