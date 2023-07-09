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
use spacepackets::ecss::PusError;
use spacepackets::tc::PusTc;
use spacepackets::tm::PusTm;
use spacepackets::{ByteConversionError, SizeMissmatch, SpHeader};
use std::error::Error;

pub mod event;
pub mod event_man;
pub mod event_srv;
pub mod hk;
pub mod mode;
pub mod scheduler;
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
    Direct(PusTm<'tm>),
}

impl From<StoreAddr> for PusTmWrapper<'_> {
    fn from(value: StoreAddr) -> Self {
        Self::InStore(value)
    }
}

impl<'tm> From<PusTm<'tm>> for PusTmWrapper<'tm> {
    fn from(value: PusTm<'tm>) -> Self {
        Self::Direct(value)
    }
}

pub type TcAddrWithToken = (StoreAddr, TcStateToken);

/// Generic abstraction for a telecommand being sent around after is has been accepted.
/// The actual telecommand is stored inside a pre-allocated pool structure.
pub type AcceptedTc = (StoreAddr, VerificationToken<TcStateAccepted>);

/// Generic error type for sending something via a message queue.
#[derive(Debug, Copy, Clone)]
pub enum GenericSendError {
    RxDisconnected,
    QueueFull(u32),
}

impl Display for GenericSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            GenericSendError::RxDisconnected => {
                write!(f, "rx side has disconnected")
            }
            GenericSendError::QueueFull(max_cap) => {
                write!(f, "queue with max capacity of {max_cap} is full")
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
    fn send_tc(&self, tc: PusTc, token: Option<TcStateToken>) -> Result<(), EcssTmtcError>;
}

pub struct ReceivedTcWrapper {
    pub store_addr: StoreAddr,
    pub token: Option<TcStateToken>,
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
    fn recv_tc(&self) -> Result<ReceivedTcWrapper, TryRecvTmtcError>;
}

/// Generic trait for objects which can receive ECSS PUS telecommands. This trait is
/// implemented by the [crate::tmtc::pus_distrib::PusDistributor] objects to allow passing PUS TC
/// packets into it.
pub trait ReceivesEcssPusTc {
    type Error;
    fn pass_pus_tc(&mut self, header: &SpHeader, pus_tc: &PusTc) -> Result<(), Self::Error>;
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
        EcssChannel, EcssTcReceiver, EcssTcReceiverCore, EcssTmSender, EcssTmSenderCore,
        EcssTmtcError, GenericRecvError, GenericSendError, PusTmWrapper, ReceivedTcWrapper,
        TcAddrWithToken, TryRecvTmtcError,
    };
    use crate::tmtc::tm_helper::SharedTmStore;
    use crate::ChannelId;
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    use spacepackets::ecss::PusError;
    use spacepackets::time::cds::TimeProvider;
    use spacepackets::time::StdTimestampError;
    use spacepackets::time::TimeWriter;
    use spacepackets::tm::PusTm;
    use std::cell::RefCell;
    use std::string::String;
    use std::sync::mpsc;
    use std::sync::mpsc::{SendError, TryRecvError};
    use thiserror::Error;

    #[derive(Clone)]
    pub struct MpscTmInStoreSender {
        id: ChannelId,
        name: &'static str,
        shared_tm_store: SharedTmStore,
        sender: mpsc::Sender<StoreAddr>,
        pub ignore_poison_errors: bool,
    }

    impl EcssChannel for MpscTmInStoreSender {
        fn id(&self) -> ChannelId {
            self.id
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl From<SendError<StoreAddr>> for EcssTmtcError {
        fn from(_: SendError<StoreAddr>) -> Self {
            Self::Send(GenericSendError::RxDisconnected)
        }
    }
    impl MpscTmInStoreSender {
        pub fn send_direct_tm(&self, tm: PusTm) -> Result<(), EcssTmtcError> {
            let addr = self.shared_tm_store.add_pus_tm(&tm)?;
            self.sender
                .send(addr)
                .map_err(|_| EcssTmtcError::Send(GenericSendError::RxDisconnected))
        }
    }

    impl EcssTmSenderCore for MpscTmInStoreSender {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmWrapper::InStore(addr) => self.sender.send(addr).map_err(|e| e.into()),
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
                ignore_poison_errors: false,
            }
        }
    }

    pub struct MpscTcInStoreReceiver {
        id: ChannelId,
        name: &'static str,
        receiver: mpsc::Receiver<TcAddrWithToken>,
        pub ignore_poison_errors: bool,
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
        fn recv_tc(&self) -> Result<ReceivedTcWrapper, TryRecvTmtcError> {
            let (store_addr, token) = self.receiver.try_recv().map_err(|e| match e {
                TryRecvError::Empty => TryRecvTmtcError::Empty,
                TryRecvError::Disconnected => {
                    TryRecvTmtcError::Error(EcssTmtcError::from(GenericRecvError::TxDisconnected))
                }
            })?;
            Ok(ReceivedTcWrapper {
                store_addr,
                token: Some(token),
            })
        }
    }

    impl MpscTcInStoreReceiver {
        pub fn new(
            id: ChannelId,
            name: &'static str,
            receiver: mpsc::Receiver<TcAddrWithToken>,
        ) -> Self {
            Self {
                id,
                name,
                receiver,
                ignore_poison_errors: false,
            }
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

    impl From<SendError<Vec<u8>>> for EcssTmtcError {
        fn from(_: SendError<Vec<u8>>) -> Self {
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
        #[error("invalid application data")]
        InvalidAppData(String),
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

    /// Base class for handlers which can handle PUS TC packets. Right now, the verification
    /// reporter is constrained to the [StdVerifReporterWithSender] and the service handler
    /// relies on TMTC packets being exchanged via a [SharedPool].
    pub struct PusServiceBase {
        pub tc_receiver: Box<dyn EcssTcReceiver>,
        pub shared_tc_store: SharedPool,
        pub tm_sender: Box<dyn EcssTmSender>,
        pub tm_apid: u16,
        /// The verification handler is wrapped in a [RefCell] to allow the interior mutability
        /// pattern. This makes writing methods which are not mutable a lot easier.
        pub verification_handler: RefCell<StdVerifReporterWithSender>,
        pub pus_buf: [u8; 2048],
        pub pus_size: usize,
    }

    impl PusServiceBase {
        pub fn new(
            tc_receiver: Box<dyn EcssTcReceiver>,
            shared_tc_store: SharedPool,
            tm_sender: Box<dyn EcssTmSender>,
            tm_apid: u16,
            verification_handler: StdVerifReporterWithSender,
        ) -> Self {
            Self {
                tc_receiver,
                shared_tc_store,
                tm_apid,
                tm_sender,
                verification_handler: RefCell::new(verification_handler),
                pus_buf: [0; 2048],
                pus_size: 0,
            }
        }

        pub fn get_current_timestamp(
            &self,
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

        pub fn get_current_timestamp_ignore_error(&self) -> [u8; 7] {
            let mut dummy = None;
            self.get_current_timestamp(&mut dummy)
        }
    }

    pub trait PusServiceHandler {
        fn psb_mut(&mut self) -> &mut PusServiceBase;
        fn psb(&self) -> &PusServiceBase;
        fn handle_one_tc(
            &mut self,
            addr: StoreAddr,
            token: VerificationToken<TcStateAccepted>,
        ) -> Result<PusPacketHandlerResult, PusPacketHandlingError>;

        fn copy_tc_to_buf(&mut self, addr: StoreAddr) -> Result<(), PusPacketHandlingError> {
            // Keep locked section as short as possible.
            let psb_mut = self.psb_mut();
            let mut tc_pool = psb_mut
                .shared_tc_store
                .write()
                .map_err(|_| PusPacketHandlingError::EcssTmtc(EcssTmtcError::StoreLock))?;
            let tc_guard = tc_pool.read_with_guard(addr);
            let tc_raw = tc_guard.read().unwrap();
            psb_mut.pus_buf[0..tc_raw.len()].copy_from_slice(tc_raw);
            Ok(())
        }

        fn handle_next_packet(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
            match self.psb().tc_receiver.recv_tc() {
                Ok(ReceivedTcWrapper { store_addr, token }) => {
                    if token.is_none() {
                        return Err(PusPacketHandlingError::InvalidVerificationToken);
                    }
                    let token = token.unwrap();
                    let accepted_token = VerificationToken::<TcStateAccepted>::try_from(token)
                        .map_err(|_| PusPacketHandlingError::InvalidVerificationToken)?;
                    self.handle_one_tc(store_addr, accepted_token)
                }
                Err(e) => match e {
                    TryRecvTmtcError::Error(e) => Err(PusPacketHandlingError::EcssTmtc(e)),
                    TryRecvTmtcError::Empty => Ok(PusPacketHandlerResult::Empty),
                },
            }
        }
    }
}

pub(crate) fn source_buffer_large_enough(cap: usize, len: usize) -> Result<(), EcssTmtcError> {
    if len > cap {
        return Err(
            PusError::ByteConversion(ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                found: cap,
                expected: len,
            }))
            .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use spacepackets::tm::{GenericPusTmSecondaryHeader, PusTm};
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
        pub fn new_from_tm(tm: &PusTm) -> Self {
            let mut time_stamp = [0; 7];
            time_stamp.clone_from_slice(&tm.timestamp().unwrap()[0..7]);
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
