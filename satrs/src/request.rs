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

use crate::{ChannelId, TargetId};

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

pub struct MessageWithSenderId<MESSAGE> {
    pub sender_id: ChannelId,
    pub message: MESSAGE,
}

impl<MESSAGE> MessageWithSenderId<MESSAGE> {
    pub fn new(sender_id: ChannelId, message: MESSAGE) -> Self {
        Self { sender_id, message }
    }
}

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
mod std_mod {
    use std::sync::mpsc;

    use hashbrown::HashMap;

    use crate::{
        queue::{GenericReceiveError, GenericSendError, GenericTargetedMessagingError},
        ChannelId,
    };

    use super::MessageWithSenderId;

    pub struct MpscMessageSenderMap<MESSAGE>(
        pub HashMap<ChannelId, mpsc::Sender<MessageWithSenderId<MESSAGE>>>,
    );

    impl<MESSAGE> Default for MpscMessageSenderMap<MESSAGE> {
        fn default() -> Self {
            Self(Default::default())
        }
    }

    impl<MESSAGE> MpscMessageSenderMap<MESSAGE> {
        pub fn send_message(
            &self,
            local_channel_id: ChannelId,
            target_channel_id: ChannelId,
            message: MESSAGE,
        ) -> Result<(), GenericTargetedMessagingError> {
            if self.0.contains_key(&target_channel_id) {
                self.0
                    .get(&target_channel_id)
                    .unwrap()
                    .send(MessageWithSenderId::new(local_channel_id, message))
                    .map_err(|_| GenericSendError::RxDisconnected)?;
                return Ok(());
            }
            Err(GenericTargetedMessagingError::TargetDoesNotExist(
                target_channel_id,
            ))
        }

        pub fn add_message_target(
            &mut self,
            target_id: ChannelId,
            message_sender: mpsc::Sender<MessageWithSenderId<MESSAGE>>,
        ) {
            self.0.insert(target_id, message_sender);
        }
    }

    pub struct MpscMessageSenderWithId<MESSAGE> {
        pub local_channel_id: ChannelId,
        pub message_sender_map: MpscMessageSenderMap<MESSAGE>,
    }

    impl<MESSAGE> MpscMessageSenderWithId<MESSAGE> {
        pub fn new(local_channel_id: ChannelId) -> Self {
            Self {
                local_channel_id,
                message_sender_map: Default::default(),
            }
        }

        pub fn send_message(
            &self,
            target_channel_id: ChannelId,
            message: MESSAGE,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.message_sender_map
                .send_message(self.local_channel_id, target_channel_id, message)
        }

        pub fn add_message_target(
            &mut self,
            target_id: ChannelId,
            message_sender: mpsc::Sender<MessageWithSenderId<MESSAGE>>,
        ) {
            self.message_sender_map
                .add_message_target(target_id, message_sender)
        }
    }

    pub struct MpscMessageWithSenderReceiver<MESSAGE>(
        pub mpsc::Receiver<MessageWithSenderId<MESSAGE>>,
    );

    impl<MESSAGE> From<mpsc::Receiver<MessageWithSenderId<MESSAGE>>>
        for MpscMessageWithSenderReceiver<MESSAGE>
    {
        fn from(value: mpsc::Receiver<MessageWithSenderId<MESSAGE>>) -> Self {
            Self(value)
        }
    }

    impl<MESSAGE> MpscMessageWithSenderReceiver<MESSAGE> {
        pub fn try_recv_message(
            &self,
            local_id: ChannelId,
        ) -> Result<Option<MessageWithSenderId<MESSAGE>>, GenericTargetedMessagingError> {
            match self.0.try_recv() {
                Ok(reply) => {
                    return Ok(Some(reply));
                }
                Err(e) => {
                    if e == mpsc::TryRecvError::Disconnected {
                        return Err(GenericReceiveError::TxDisconnected(Some(local_id)).into());
                    }
                }
            }
            Ok(None)
        }
    }

    pub struct MpscMessageReceiverWithIds<MESSAGE> {
        local_channel_id: ChannelId,
        reply_receiver: MpscMessageWithSenderReceiver<MESSAGE>,
    }

    impl<MESSAGE> MpscMessageReceiverWithIds<MESSAGE> {
        pub fn new(
            local_channel_id: ChannelId,
            reply_receiver: MpscMessageWithSenderReceiver<MESSAGE>,
        ) -> Self {
            Self {
                local_channel_id,
                reply_receiver,
            }
        }

        pub fn local_channel_id(&self) -> ChannelId {
            self.local_channel_id
        }

        pub fn try_recv_message(
            &self,
        ) -> Result<Option<MessageWithSenderId<MESSAGE>>, GenericTargetedMessagingError> {
            match self.reply_receiver.0.try_recv() {
                Ok(reply) => {
                    return Ok(Some(reply));
                }
                Err(e) => {
                    if e == mpsc::TryRecvError::Disconnected {
                        return Err(GenericReceiveError::TxDisconnected(Some(
                            self.local_channel_id(),
                        ))
                        .into());
                    }
                }
            }
            Ok(None)
        }
    }

    pub struct MpscMessageSenderAndReceiver<TO, FROM> {
        pub local_channel_id: ChannelId,
        pub message_sender_map: MpscMessageSenderMap<TO>,
        pub message_receiver: MpscMessageWithSenderReceiver<FROM>,
    }

    impl<TO, FROM> MpscMessageSenderAndReceiver<TO, FROM> {
        pub fn new(
            local_channel_id: ChannelId,
            message_receiver: mpsc::Receiver<MessageWithSenderId<FROM>>,
        ) -> Self {
            Self {
                local_channel_id,
                message_sender_map: Default::default(),
                message_receiver: MpscMessageWithSenderReceiver::from(message_receiver),
            }
        }

        pub fn add_message_target(
            &mut self,
            target_id: ChannelId,
            message_sender: mpsc::Sender<MessageWithSenderId<TO>>,
        ) {
            self.message_sender_map
                .add_message_target(target_id, message_sender)
        }

        pub fn local_channel_id_generic(&self) -> ChannelId {
            self.local_channel_id
        }
    }

    pub struct MpscRequestAndReplySenderAndReceiver<REQUEST, REPLY> {
        pub local_channel_id: ChannelId,
        // These 2 are a functional group.
        pub request_sender_map: MpscMessageSenderMap<REQUEST>,
        pub reply_receiver: MpscMessageWithSenderReceiver<REPLY>,
        // These 2 are a functional group.
        pub request_receiver: MpscMessageWithSenderReceiver<REQUEST>,
        pub reply_sender_map: MpscMessageSenderMap<REPLY>,
    }

    impl<REQUEST, REPLY> MpscRequestAndReplySenderAndReceiver<REQUEST, REPLY> {
        pub fn new(
            local_channel_id: ChannelId,
            request_receiver: mpsc::Receiver<MessageWithSenderId<REQUEST>>,
            reply_receiver: mpsc::Receiver<MessageWithSenderId<REPLY>>,
        ) -> Self {
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
