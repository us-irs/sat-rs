//! # PUS support modules
//!
//! This module contains structures to make working with the PUS C standard easier.
//! The satrs-example application contains various usage examples of these components.
use crate::ChannelId;
use core::fmt::{Display, Formatter};
#[cfg(feature = "alloc")]
use downcast_rs::{impl_downcast, Downcast};
#[cfg(feature = "alloc")]
use dyn_clone::DynClone;
#[cfg(feature = "std")]
use std::error::Error;

use spacepackets::ecss::tc::{PusTcCreator, PusTcReader};
use spacepackets::ecss::tm::PusTmCreator;
use spacepackets::ecss::PusError;
use spacepackets::{ByteConversionError, SpHeader};

pub mod event;
pub mod event_man;
#[cfg(feature = "std")]
pub mod event_srv;
pub mod hk;
pub mod mode;
pub mod scheduler;
#[cfg(feature = "std")]
pub mod scheduler_srv;
#[cfg(feature = "std")]
pub mod test;
pub mod verification;

#[cfg(feature = "alloc")]
pub use alloc_mod::*;

use crate::pool::{StoreAddr, StoreError};
use crate::pus::verification::{TcStateAccepted, TcStateToken, VerificationToken};
#[cfg(feature = "std")]
pub use std_mod::*;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PusTmWrapper<'tm> {
    InStore(StoreAddr),
    Direct(PusTmCreator<'tm>),
}

impl From<StoreAddr> for PusTmWrapper<'_> {
    fn from(value: StoreAddr) -> Self {
        Self::InStore(value)
    }
}

impl<'tm> From<PusTmCreator<'tm>> for PusTmWrapper<'tm> {
    fn from(value: PusTmCreator<'tm>) -> Self {
        Self::Direct(value)
    }
}

/// Generic error type for sending something via a message queue.
#[derive(Debug, Copy, Clone)]
pub enum GenericSendError {
    RxDisconnected,
    QueueFull(Option<u32>),
}

impl Display for GenericSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            GenericSendError::RxDisconnected => {
                write!(f, "rx side has disconnected")
            }
            GenericSendError::QueueFull(max_cap) => {
                write!(f, "queue with max capacity of {max_cap:?} is full")
            }
        }
    }
}

#[cfg(feature = "std")]
impl Error for GenericSendError {}

/// Generic error type for sending something via a message queue.
#[derive(Debug, Copy, Clone)]
pub enum GenericRecvError {
    Empty,
    TxDisconnected,
}

impl Display for GenericRecvError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TxDisconnected => {
                write!(f, "tx side has disconnected")
            }
            Self::Empty => {
                write!(f, "nothing to receive")
            }
        }
    }
}

#[cfg(feature = "std")]
impl Error for GenericRecvError {}

#[derive(Debug, Clone)]
pub enum EcssTmtcError {
    StoreLock,
    Store(StoreError),
    Pus(PusError),
    CantSendAddr(StoreAddr),
    Send(GenericSendError),
    Recv(GenericRecvError),
}

impl Display for EcssTmtcError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            EcssTmtcError::StoreLock => {
                write!(f, "store lock error")
            }
            EcssTmtcError::Store(store) => {
                write!(f, "store error: {store}")
            }
            EcssTmtcError::Pus(pus_e) => {
                write!(f, "PUS error: {pus_e}")
            }
            EcssTmtcError::CantSendAddr(addr) => {
                write!(f, "can not send address {addr}")
            }
            EcssTmtcError::Send(send_e) => {
                write!(f, "send error {send_e}")
            }
            EcssTmtcError::Recv(recv_e) => {
                write!(f, "recv error {recv_e}")
            }
        }
    }
}

impl From<StoreError> for EcssTmtcError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

impl From<PusError> for EcssTmtcError {
    fn from(value: PusError) -> Self {
        Self::Pus(value)
    }
}

impl From<GenericSendError> for EcssTmtcError {
    fn from(value: GenericSendError) -> Self {
        Self::Send(value)
    }
}

impl From<GenericRecvError> for EcssTmtcError {
    fn from(value: GenericRecvError) -> Self {
        Self::Recv(value)
    }
}

#[cfg(feature = "std")]
impl Error for EcssTmtcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            EcssTmtcError::Store(e) => Some(e),
            EcssTmtcError::Pus(e) => Some(e),
            EcssTmtcError::Send(e) => Some(e),
            _ => None,
        }
    }
}
pub trait EcssChannel: Send {
    /// Each sender can have an ID associated with it
    fn id(&self) -> ChannelId;
    fn name(&self) -> &'static str {
        "unset"
    }
}

/// Generic trait for a user supplied sender object.
///
/// This sender object is responsible for sending PUS telemetry to a TM sink.
pub trait EcssTmSenderCore: EcssChannel {
    fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError>;
}

/// Generic trait for a user supplied sender object.
///
/// This sender object is responsible for sending PUS telecommands to a TC recipient. Each
/// telecommand can optionally have a token which contains its verification state.
pub trait EcssTcSenderCore: EcssChannel {
    fn send_tc(&self, tc: PusTcCreator, token: Option<TcStateToken>) -> Result<(), EcssTmtcError>;
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TcInMemory {
    StoreAddr(StoreAddr),
    #[cfg(feature = "alloc")]
    Vec(alloc::vec::Vec<u8>),
}

impl From<StoreAddr> for TcInMemory {
    fn from(value: StoreAddr) -> Self {
        Self::StoreAddr(value)
    }
}

#[cfg(feature = "alloc")]
impl From<alloc::vec::Vec<u8>> for TcInMemory {
    fn from(value: alloc::vec::Vec<u8>) -> Self {
        Self::Vec(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcssTcAndToken {
    pub tc_in_memory: TcInMemory,
    pub token: Option<TcStateToken>,
}

impl EcssTcAndToken {
    pub fn new(tc_in_memory: impl Into<TcInMemory>, token: impl Into<TcStateToken>) -> Self {
        Self {
            tc_in_memory: tc_in_memory.into(),
            token: Some(token.into()),
        }
    }
}

/// Generic abstraction for a telecommand being sent around after is has been accepted.
pub struct AcceptedEcssTcAndToken {
    pub tc_in_memory: TcInMemory,
    pub token: VerificationToken<TcStateAccepted>,
}

impl From<AcceptedEcssTcAndToken> for EcssTcAndToken {
    fn from(value: AcceptedEcssTcAndToken) -> Self {
        EcssTcAndToken {
            tc_in_memory: value.tc_in_memory,
            token: Some(value.token.into()),
        }
    }
}

impl TryFrom<EcssTcAndToken> for AcceptedEcssTcAndToken {
    type Error = ();

    fn try_from(value: EcssTcAndToken) -> Result<Self, Self::Error> {
        if let Some(TcStateToken::Accepted(token)) = value.token {
            return Ok(AcceptedEcssTcAndToken {
                tc_in_memory: value.tc_in_memory,
                token,
            });
        }
        Err(())
    }
}

#[derive(Debug, Clone)]
pub enum TryRecvTmtcError {
    Error(EcssTmtcError),
    Empty,
}

impl From<EcssTmtcError> for TryRecvTmtcError {
    fn from(value: EcssTmtcError) -> Self {
        Self::Error(value)
    }
}

impl From<PusError> for TryRecvTmtcError {
    fn from(value: PusError) -> Self {
        Self::Error(value.into())
    }
}

impl From<StoreError> for TryRecvTmtcError {
    fn from(value: StoreError) -> Self {
        Self::Error(value.into())
    }
}

/// Generic trait for a user supplied receiver object.
pub trait EcssTcReceiverCore: EcssChannel {
    fn recv_tc(&self) -> Result<EcssTcAndToken, TryRecvTmtcError>;
}

/// Generic trait for objects which can receive ECSS PUS telecommands. This trait is
/// implemented by the [crate::tmtc::pus_distrib::PusDistributor] objects to allow passing PUS TC
/// packets into it. It is generally assumed that the telecommand is stored in some pool structure,
/// and the store address is passed as well. This allows efficient zero-copy forwarding of
/// telecommands.
pub trait ReceivesEcssPusTc {
    type Error;
    fn pass_pus_tc(&mut self, header: &SpHeader, pus_tc: &PusTcReader) -> Result<(), Self::Error>;
}

#[cfg(feature = "alloc")]
mod alloc_mod {
    use super::*;

    /// Extension trait for [EcssTmSenderCore].
    ///
    /// It provides additional functionality, for example by implementing the [Downcast] trait
    /// and the [DynClone] trait.
    ///
    /// [Downcast] is implemented to allow passing the sender as a boxed trait object and still
    /// retrieve the concrete type at a later point.
    ///
    /// [DynClone] allows cloning the trait object as long as the boxed object implements
    /// [Clone].
    #[cfg(feature = "alloc")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
    pub trait EcssTmSender: EcssTmSenderCore + Downcast + DynClone {}

    /// Blanket implementation for all types which implement [EcssTmSenderCore] and are clonable.
    impl<T> EcssTmSender for T where T: EcssTmSenderCore + Clone + 'static {}

    dyn_clone::clone_trait_object!(EcssTmSender);
    impl_downcast!(EcssTmSender);

    /// Extension trait for [EcssTcSenderCore].
    ///
    /// It provides additional functionality, for example by implementing the [Downcast] trait
    /// and the [DynClone] trait.
    ///
    /// [Downcast] is implemented to allow passing the sender as a boxed trait object and still
    /// retrieve the concrete type at a later point.
    ///
    /// [DynClone] allows cloning the trait object as long as the boxed object implements
    /// [Clone].
    #[cfg(feature = "alloc")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
    pub trait EcssTcSender: EcssTcSenderCore + Downcast + DynClone {}

    /// Blanket implementation for all types which implement [EcssTcSenderCore] and are clonable.
    impl<T> EcssTcSender for T where T: EcssTcSenderCore + Clone + 'static {}

    dyn_clone::clone_trait_object!(EcssTcSender);
    impl_downcast!(EcssTcSender);

    /// Extension trait for [EcssTcReceiverCore].
    ///
    /// It provides additional functionality, for example by implementing the [Downcast] trait
    /// and the [DynClone] trait.
    ///
    /// [Downcast] is implemented to allow passing the sender as a boxed trait object and still
    /// retrieve the concrete type at a later point.
    ///
    /// [DynClone] allows cloning the trait object as long as the boxed object implements
    /// [Clone].
    #[cfg(feature = "alloc")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
    pub trait EcssTcReceiver: EcssTcReceiverCore + Downcast {}

    /// Blanket implementation for all types which implement [EcssTcReceiverCore] and are clonable.
    impl<T> EcssTcReceiver for T where T: EcssTcReceiverCore + 'static {}

    impl_downcast!(EcssTcReceiver);
}

#[cfg(feature = "std")]
pub mod std_mod {
    use crate::pool::{SharedPool, StoreAddr};
    use crate::pus::verification::{
        StdVerifReporterWithSender, TcStateAccepted, VerificationToken,
    };
    use crate::pus::{
        EcssChannel, EcssTcAndToken, EcssTcReceiver, EcssTcReceiverCore, EcssTmSender,
        EcssTmSenderCore, EcssTmtcError, GenericRecvError, GenericSendError, PusTmWrapper,
        TryRecvTmtcError,
    };
    use crate::tmtc::tm_helper::SharedTmStore;
    use crate::ChannelId;
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    use crossbeam_channel as cb;
    use spacepackets::ecss::tc::PusTcReader;
    use spacepackets::ecss::tm::PusTmCreator;
    use spacepackets::ecss::PusError;
    use spacepackets::time::cds::TimeProvider;
    use spacepackets::time::StdTimestampError;
    use spacepackets::time::TimeWriter;
    use std::cell::RefCell;
    use std::string::String;
    use std::sync::mpsc;
    use std::sync::mpsc::TryRecvError;
    use thiserror::Error;

    use super::verification::VerificationReporterWithSender;
    use super::{AcceptedEcssTcAndToken, TcInMemory};

    impl From<mpsc::SendError<StoreAddr>> for EcssTmtcError {
        fn from(_: mpsc::SendError<StoreAddr>) -> Self {
            Self::Send(GenericSendError::RxDisconnected)
        }
    }

    impl From<cb::SendError<StoreAddr>> for EcssTmtcError {
        fn from(_: cb::SendError<StoreAddr>) -> Self {
            Self::Send(GenericSendError::RxDisconnected)
        }
    }

    impl From<cb::TrySendError<StoreAddr>> for EcssTmtcError {
        fn from(value: cb::TrySendError<StoreAddr>) -> Self {
            match value {
                cb::TrySendError::Full(_) => Self::Send(GenericSendError::QueueFull(None)),
                cb::TrySendError::Disconnected(_) => Self::Send(GenericSendError::RxDisconnected),
            }
        }
    }

    #[derive(Clone)]
    pub struct MpscTmInStoreSender {
        id: ChannelId,
        name: &'static str,
        shared_tm_store: SharedTmStore,
        sender: mpsc::Sender<StoreAddr>,
    }

    impl EcssChannel for MpscTmInStoreSender {
        fn id(&self) -> ChannelId {
            self.id
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl MpscTmInStoreSender {
        pub fn send_direct_tm(&self, tm: PusTmCreator) -> Result<(), EcssTmtcError> {
            let addr = self.shared_tm_store.add_pus_tm(&tm)?;
            self.sender
                .send(addr)
                .map_err(|_| EcssTmtcError::Send(GenericSendError::RxDisconnected))
        }
    }

    impl EcssTmSenderCore for MpscTmInStoreSender {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmWrapper::InStore(addr) => {
                    self.sender.send(addr)?;
                    Ok(())
                }
                PusTmWrapper::Direct(tm) => self.send_direct_tm(tm),
            }
        }
    }

    impl MpscTmInStoreSender {
        pub fn new(
            id: ChannelId,
            name: &'static str,
            shared_tm_store: SharedTmStore,
            sender: mpsc::Sender<StoreAddr>,
        ) -> Self {
            Self {
                id,
                name,
                shared_tm_store,
                sender,
            }
        }
    }

    pub struct MpscTcInStoreReceiver {
        id: ChannelId,
        name: &'static str,
        receiver: mpsc::Receiver<EcssTcAndToken>,
    }

    impl EcssChannel for MpscTcInStoreReceiver {
        fn id(&self) -> ChannelId {
            self.id
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl EcssTcReceiverCore for MpscTcInStoreReceiver {
        fn recv_tc(&self) -> Result<EcssTcAndToken, TryRecvTmtcError> {
            self.receiver.try_recv().map_err(|e| match e {
                TryRecvError::Empty => TryRecvTmtcError::Empty,
                TryRecvError::Disconnected => {
                    TryRecvTmtcError::Error(EcssTmtcError::from(GenericRecvError::TxDisconnected))
                }
            })
        }
    }

    impl MpscTcInStoreReceiver {
        pub fn new(
            id: ChannelId,
            name: &'static str,
            receiver: mpsc::Receiver<EcssTcAndToken>,
        ) -> Self {
            Self { id, name, receiver }
        }
    }

    /// This class can be used if frequent heap allocations during run-time are not an issue.
    /// PUS TM packets will be sent around as [Vec]s. Please note that the current implementation
    /// of this class can not deal with store addresses, so it is assumed that is is always
    /// going to be called with direct packets.
    #[derive(Clone)]
    pub struct MpscTmAsVecSender {
        id: ChannelId,
        sender: mpsc::Sender<Vec<u8>>,
        name: &'static str,
    }

    impl From<mpsc::SendError<Vec<u8>>> for EcssTmtcError {
        fn from(_: mpsc::SendError<Vec<u8>>) -> Self {
            Self::Send(GenericSendError::RxDisconnected)
        }
    }

    impl MpscTmAsVecSender {
        pub fn new(id: u32, name: &'static str, sender: mpsc::Sender<Vec<u8>>) -> Self {
            Self { id, sender, name }
        }
    }

    impl EcssChannel for MpscTmAsVecSender {
        fn id(&self) -> ChannelId {
            self.id
        }
        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl EcssTmSenderCore for MpscTmAsVecSender {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmWrapper::InStore(addr) => Err(EcssTmtcError::CantSendAddr(addr)),
                PusTmWrapper::Direct(tm) => {
                    let mut vec = Vec::new();
                    tm.append_to_vec(&mut vec).map_err(EcssTmtcError::Pus)?;
                    self.sender.send(vec)?;
                    Ok(())
                }
            }
        }
    }

    #[derive(Clone)]
    pub struct CrossbeamTmInStoreSender {
        id: ChannelId,
        name: &'static str,
        shared_tm_store: SharedTmStore,
        sender: crossbeam_channel::Sender<StoreAddr>,
    }

    impl CrossbeamTmInStoreSender {
        pub fn new(
            id: ChannelId,
            name: &'static str,
            shared_tm_store: SharedTmStore,
            sender: crossbeam_channel::Sender<StoreAddr>,
        ) -> Self {
            Self {
                id,
                name,
                shared_tm_store,
                sender,
            }
        }
    }

    impl EcssChannel for CrossbeamTmInStoreSender {
        fn id(&self) -> ChannelId {
            self.id
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl EcssTmSenderCore for CrossbeamTmInStoreSender {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmWrapper::InStore(addr) => self.sender.try_send(addr)?,
                PusTmWrapper::Direct(tm) => {
                    let addr = self.shared_tm_store.add_pus_tm(&tm)?;
                    self.sender.try_send(addr)?;
                }
            }
            Ok(())
        }
    }

    pub struct CrossbeamTcInStoreReceiver {
        id: ChannelId,
        name: &'static str,
        receiver: cb::Receiver<EcssTcAndToken>,
    }

    impl CrossbeamTcInStoreReceiver {
        pub fn new(
            id: ChannelId,
            name: &'static str,
            receiver: cb::Receiver<EcssTcAndToken>,
        ) -> Self {
            Self { id, name, receiver }
        }
    }

    impl EcssChannel for CrossbeamTcInStoreReceiver {
        fn id(&self) -> ChannelId {
            self.id
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl EcssTcReceiverCore for CrossbeamTcInStoreReceiver {
        fn recv_tc(&self) -> Result<EcssTcAndToken, TryRecvTmtcError> {
            self.receiver.try_recv().map_err(|e| match e {
                cb::TryRecvError::Empty => TryRecvTmtcError::Empty,
                cb::TryRecvError::Disconnected => {
                    TryRecvTmtcError::Error(EcssTmtcError::from(GenericRecvError::TxDisconnected))
                }
            })
        }
    }

    #[derive(Debug, Clone, Error)]
    pub enum PusPacketHandlingError {
        #[error("generic PUS error: {0}")]
        Pus(#[from] PusError),
        #[error("wrong service number {0} for packet handler")]
        WrongService(u8),
        #[error("invalid subservice {0}")]
        InvalidSubservice(u8),
        #[error("not enough application data available: {0}")]
        NotEnoughAppData(String),
        #[error("PUS packet too large, does not fit in buffer: {0}")]
        PusPacketTooLarge(usize),
        #[error("invalid application data")]
        InvalidAppData(String),
        #[error("invalid format of TC in memory: {0:?}")]
        InvalidTcInMemoryFormat(TcInMemory),
        #[error("generic ECSS tmtc error: {0}")]
        EcssTmtc(#[from] EcssTmtcError),
        #[error("invalid verification token")]
        InvalidVerificationToken,
        #[error("other error {0}")]
        Other(String),
    }

    #[derive(Debug, Clone, Error)]
    pub enum PartialPusHandlingError {
        #[error("generic timestamp generation error")]
        Time(#[from] StdTimestampError),
        #[error("error sending telemetry: {0}")]
        TmSend(#[from] EcssTmtcError),
        #[error("error sending verification message")]
        Verification,
        #[error("invalid verification token")]
        NoVerificationToken,
    }

    /// Generic result type for handlers which can process PUS packets.
    #[derive(Debug, Clone)]
    pub enum PusPacketHandlerResult {
        RequestHandled,
        RequestHandledPartialSuccess(PartialPusHandlingError),
        SubserviceNotImplemented(u8, VerificationToken<TcStateAccepted>),
        CustomSubservice(u8, VerificationToken<TcStateAccepted>),
        Empty,
    }

    impl From<PartialPusHandlingError> for PusPacketHandlerResult {
        fn from(value: PartialPusHandlingError) -> Self {
            Self::RequestHandledPartialSuccess(value)
        }
    }

    pub trait EcssTcInMemConverter {
        fn cache_ecss_tc_in_memory<'a>(
            &'a mut self,
            possible_packet: &'a AcceptedEcssTcAndToken,
        ) -> Result<(), PusPacketHandlingError>;

        fn tc_slice_raw(&self) -> &[u8];

        fn convert_ecss_tc_in_memory_to_reader<'a>(
            &'a mut self,
            possible_packet: &'a AcceptedEcssTcAndToken,
        ) -> Result<PusTcReader<'a>, PusPacketHandlingError> {
            self.cache_ecss_tc_in_memory(possible_packet)?;
            Ok(PusTcReader::new(self.tc_slice_raw())?.0)
        }
    }

    pub struct EcssTcInVecConverter {
        pub pus_tc_raw: Option<Vec<u8>>,
    }

    impl EcssTcInMemConverter for EcssTcInVecConverter {
        fn cache_ecss_tc_in_memory<'a>(
            &'a mut self,
            possible_packet: &'a AcceptedEcssTcAndToken,
        ) -> Result<(), PusPacketHandlingError> {
            self.pus_tc_raw = None;
            match &possible_packet.tc_in_memory {
                super::TcInMemory::StoreAddr(_) => {
                    return Err(PusPacketHandlingError::InvalidTcInMemoryFormat(
                        possible_packet.tc_in_memory.clone(),
                    ));
                }
                super::TcInMemory::Vec(vec) => {
                    self.pus_tc_raw = Some(vec.clone());
                }
            };
            Ok(())
        }

        fn tc_slice_raw(&self) -> &[u8] {
            if self.pus_tc_raw.is_none() {
                return &[];
            }
            self.pus_tc_raw.as_ref().unwrap()
        }
    }

    pub struct EcssTcInStoreConverter {
        pub shared_tc_store: SharedPool,
        pub pus_buf: Vec<u8>,
    }

    impl EcssTcInStoreConverter {
        pub fn new(shared_tc_store: SharedPool, max_expected_tc_size: usize) -> Self {
            Self {
                shared_tc_store,
                pus_buf: alloc::vec![0; max_expected_tc_size],
            }
        }

        pub fn copy_tc_to_buf(&mut self, addr: StoreAddr) -> Result<(), PusPacketHandlingError> {
            // Keep locked section as short as possible.
            let mut tc_pool = self
                .shared_tc_store
                .write()
                .map_err(|_| PusPacketHandlingError::EcssTmtc(EcssTmtcError::StoreLock))?;
            let tc_guard = tc_pool.read_with_guard(addr);
            let tc_raw = tc_guard.read().unwrap();
            if tc_raw.len() > self.pus_buf.len() {
                return Err(PusPacketHandlingError::PusPacketTooLarge(tc_raw.len()));
            }
            self.pus_buf[0..tc_raw.len()].copy_from_slice(tc_raw);
            Ok(())
        }
    }

    impl EcssTcInMemConverter for EcssTcInStoreConverter {
        fn cache_ecss_tc_in_memory<'a>(
            &'a mut self,
            possible_packet: &'a AcceptedEcssTcAndToken,
        ) -> Result<(), PusPacketHandlingError> {
            match &possible_packet.tc_in_memory {
                super::TcInMemory::StoreAddr(addr) => {
                    self.copy_tc_to_buf(*addr)?;
                }
                super::TcInMemory::Vec(_) => {
                    return Err(PusPacketHandlingError::InvalidTcInMemoryFormat(
                        possible_packet.tc_in_memory.clone(),
                    ));
                }
            };
            Ok(())
        }

        fn tc_slice_raw(&self) -> &[u8] {
            self.pus_buf.as_ref()
        }
    }

    pub struct PusServiceBase {
        pub tc_receiver: Box<dyn EcssTcReceiver>,
        pub tm_sender: Box<dyn EcssTmSender>,
        pub tm_apid: u16,
        /// The verification handler is wrapped in a [RefCell] to allow the interior mutability
        /// pattern. This makes writing methods which are not mutable a lot easier.
        pub verification_handler: RefCell<StdVerifReporterWithSender>,
    }

    impl PusServiceBase {
        #[cfg(feature = "std")]
        pub fn get_current_timestamp(
            partial_error: &mut Option<PartialPusHandlingError>,
        ) -> [u8; 7] {
            let mut time_stamp: [u8; 7] = [0; 7];
            let time_provider =
                TimeProvider::from_now_with_u16_days().map_err(PartialPusHandlingError::Time);
            if let Ok(time_provider) = time_provider {
                // Can't fail, we have a buffer with the exact required size.
                time_provider.write_to_bytes(&mut time_stamp).unwrap();
            } else {
                *partial_error = Some(time_provider.unwrap_err());
            }
            time_stamp
        }

        #[cfg(feature = "std")]
        pub fn get_current_timestamp_ignore_error() -> [u8; 7] {
            let mut dummy = None;
            Self::get_current_timestamp(&mut dummy)
        }
    }

    /// Base class for handlers which can handle PUS TC packets. Right now, the verification
    /// reporter is constrained to the [StdVerifReporterWithSender] and the service handler
    /// relies on TMTC packets being exchanged via a [SharedPool]. Please note that this variant
    /// of the PUS service base is not optimized for handling packets sent as a `Vec<u8>` and
    /// might perform additional copies to the internal buffer as well. The class should
    /// still behave correctly.
    pub struct PusServiceHandler<TcInMemConverter: EcssTcInMemConverter> {
        pub common: PusServiceBase,
        pub tc_in_mem_converter: TcInMemConverter,
    }

    impl<TcInMemConverter: EcssTcInMemConverter> PusServiceHandler<TcInMemConverter> {
        pub fn new(
            tc_receiver: Box<dyn EcssTcReceiver>,
            tm_sender: Box<dyn EcssTmSender>,
            tm_apid: u16,
            verification_handler: VerificationReporterWithSender,
            tc_in_mem_converter: TcInMemConverter,
        ) -> Self {
            Self {
                common: PusServiceBase {
                    tc_receiver,
                    tm_sender,
                    tm_apid,
                    verification_handler: RefCell::new(verification_handler),
                },
                tc_in_mem_converter,
            }
        }

        pub fn retrieve_and_accept_next_packet(
            &mut self,
        ) -> Result<Option<AcceptedEcssTcAndToken>, PusPacketHandlingError> {
            match self.common.tc_receiver.recv_tc() {
                Ok(EcssTcAndToken {
                    tc_in_memory,
                    token,
                }) => {
                    if token.is_none() {
                        return Err(PusPacketHandlingError::InvalidVerificationToken);
                    }
                    let token = token.unwrap();
                    let accepted_token = VerificationToken::<TcStateAccepted>::try_from(token)
                        .map_err(|_| PusPacketHandlingError::InvalidVerificationToken)?;
                    Ok(Some(AcceptedEcssTcAndToken {
                        tc_in_memory,
                        token: accepted_token,
                    }))
                }
                Err(e) => match e {
                    TryRecvTmtcError::Error(e) => Err(PusPacketHandlingError::EcssTmtc(e)),
                    TryRecvTmtcError::Empty => Ok(None),
                },
            }
        }
    }
}

pub(crate) fn source_buffer_large_enough(cap: usize, len: usize) -> Result<(), EcssTmtcError> {
    if len > cap {
        return Err(
            PusError::ByteConversion(ByteConversionError::ToSliceTooSmall {
                found: cap,
                expected: len,
            })
            .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use spacepackets::ecss::tm::{GenericPusTmSecondaryHeader, PusTmCreator};
    use spacepackets::CcsdsPacket;

    #[derive(Debug, Eq, PartialEq, Clone)]
    pub(crate) struct CommonTmInfo {
        pub subservice: u8,
        pub apid: u16,
        pub msg_counter: u16,
        pub dest_id: u16,
        pub time_stamp: [u8; 7],
    }

    impl CommonTmInfo {
        pub fn new_from_tm(tm: &PusTmCreator) -> Self {
            let mut time_stamp = [0; 7];
            time_stamp.clone_from_slice(&tm.timestamp()[0..7]);
            Self {
                subservice: tm.subservice(),
                apid: tm.apid(),
                msg_counter: tm.msg_counter(),
                dest_id: tm.dest_id(),
                time_stamp,
            }
        }
    }
}
