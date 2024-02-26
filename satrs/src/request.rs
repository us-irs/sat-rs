use core::fmt;
#[cfg(feature = "std")]
use std::error::Error;

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub use std_mod::*;

use spacepackets::{
    ecss::{tc::IsPusTelecommand, PusPacket},
    ByteConversionError, CcsdsPacket,
};

use crate::{queue::GenericTargetedMessagingError, ChannelId, TargetId};

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

pub struct MessageWithSenderId<MSG> {
    pub sender_id: ChannelId,
    pub message: MSG,
}

impl<MSG> MessageWithSenderId<MSG> {
    pub fn new(sender_id: ChannelId, message: MSG) -> Self {
        Self { sender_id, message }
    }
}

/// Generic trait for objects which can send targeted messages.
pub trait MessageSender<MSG>: Send {
    fn send(&self, message: MessageWithSenderId<MSG>) -> Result<(), GenericTargetedMessagingError>;
}

// Generic trait for objects which can receive targeted messages.
pub trait MessageReceiver<MSG> {
    fn try_recv(&self) -> Result<Option<MessageWithSenderId<MSG>>, GenericTargetedMessagingError>;
}

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
mod std_mod {
    use core::marker::PhantomData;
    use std::sync::mpsc;

    use hashbrown::HashMap;

    use crate::{
        queue::{GenericReceiveError, GenericSendError, GenericTargetedMessagingError},
        ChannelId,
    };

    use super::{MessageReceiver, MessageSender, MessageWithSenderId};

    impl<MSG: Send> MessageSender<MSG> for mpsc::Sender<MessageWithSenderId<MSG>> {
        fn send(
            &self,
            message: MessageWithSenderId<MSG>,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send(message)
                .map_err(|_| GenericSendError::RxDisconnected)?;
            Ok(())
        }
    }
    impl<MSG: Send> MessageSender<MSG> for mpsc::SyncSender<MessageWithSenderId<MSG>> {
        fn send(
            &self,
            message: MessageWithSenderId<MSG>,
        ) -> Result<(), GenericTargetedMessagingError> {
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

    pub struct MessageSenderMap<MSG, S: MessageSender<MSG>>(
        pub HashMap<ChannelId, S>,
        PhantomData<MSG>,
    );

    pub type MpscSenderMap<MSG> = MessageReceiverWithId<MSG, mpsc::Sender<MSG>>;
    pub type MpscBoundedSenderMap<MSG> = MessageReceiverWithId<MSG, mpsc::SyncSender<MSG>>;

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
            local_channel_id: ChannelId,
            target_channel_id: ChannelId,
            message: MSG,
        ) -> Result<(), GenericTargetedMessagingError> {
            if self.0.contains_key(&target_channel_id) {
                self.0
                    .get(&target_channel_id)
                    .unwrap()
                    .send(MessageWithSenderId::new(local_channel_id, message))
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
            target_channel_id: ChannelId,
            message: MSG,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.message_sender_map
                .send_message(self.local_channel_id, target_channel_id, message)
        }

        pub fn add_message_target(&mut self, target_id: ChannelId, message_sender: S) {
            self.message_sender_map
                .add_message_target(target_id, message_sender)
        }
    }

    impl<MSG> MessageReceiver<MSG> for mpsc::Receiver<MessageWithSenderId<MSG>> {
        fn try_recv(
            &self,
        ) -> Result<Option<MessageWithSenderId<MSG>>, GenericTargetedMessagingError> {
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

    pub struct MessageWithSenderIdReceiver<MSG, R: MessageReceiver<MSG>>(pub R, PhantomData<MSG>);

    impl<MSG, R: MessageReceiver<MSG>> From<R> for MessageWithSenderIdReceiver<MSG, R> {
        fn from(receiver: R) -> Self {
            MessageWithSenderIdReceiver(receiver, PhantomData)
        }
    }

    impl<MSG, R: MessageReceiver<MSG>> MessageWithSenderIdReceiver<MSG, R> {
        pub fn try_recv_message(
            &self,
            _local_id: ChannelId,
        ) -> Result<Option<MessageWithSenderId<MSG>>, GenericTargetedMessagingError> {
            self.0.try_recv()
        }
    }

    pub struct MessageReceiverWithId<MSG, R: MessageReceiver<MSG>> {
        local_channel_id: ChannelId,
        reply_receiver: MessageWithSenderIdReceiver<MSG, R>,
    }

    pub type MpscMessageReceiverWithId<MSG> = MessageReceiverWithId<MSG, mpsc::Receiver<MSG>>;

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
        ) -> Result<Option<MessageWithSenderId<MSG>>, GenericTargetedMessagingError> {
            self.reply_receiver.0.try_recv()
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
