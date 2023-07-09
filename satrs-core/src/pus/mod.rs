//! # PUS support modules
//!
//! This module contains structures to make working with the PUS C standard easier.
//! The satrs-example application contains various usage examples of these components.
use crate::pus::verification::TcStateToken;
use crate::SenderId;
#[cfg(feature = "alloc")]
use downcast_rs::{impl_downcast, Downcast};
#[cfg(feature = "alloc")]
use dyn_clone::DynClone;
use spacepackets::ecss::PusError;
use spacepackets::tc::PusTc;
use spacepackets::time::TimestampError;
use spacepackets::tm::PusTm;
use spacepackets::{ByteConversionError, SizeMissmatch};

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

use crate::pool::StoreAddr;
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

/// Generic error type for PUS TM handling.
#[derive(Debug, Copy, Clone)]
pub enum EcssTmtcSendError {
    RxDisconnected,
    QueueFull(u32),
}

// /// Generic error type for PUS TMTC handling.
// #[derive(Debug, Clone)]
// pub enum EcssTmtcErrorWithSend {
//     /// Errors related to the time stamp format of the telemetry
//     Timestamp(TimestampError),
//     /// Errors related to byte conversion, for example insufficient buffer size for given data
//     ByteConversion(ByteConversionError),
//     /// Errors related to PUS packet format
//     Pus(PusError),
//     Send(EcssTmtcSendError)
// }
//
// impl From<PusError> for EcssTmtcErrorWithSend {
//     fn from(e: PusError) -> Self {
//         EcssTmtcErrorWithSend::Pus(e)
//     }
// }
//
// impl From<ByteConversionError> for EcssTmtcErrorWithSend {
//     fn from(e: ByteConversionError) -> Self {
//         EcssTmtcErrorWithSend::ByteConversion(e)
//     }
// }

pub trait EcssSender: Send {
    /// Each sender can have an ID associated with it
    fn id(&self) -> SenderId;
    fn name(&self) -> &'static str {
        "unset"
    }
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
}

#[cfg(feature = "std")]
pub mod std_mod {
    use crate::pool::{ShareablePoolProvider, SharedPool, StoreAddr, StoreError};
    use crate::pus::verification::{
        StdVerifReporterWithSender, TcStateAccepted, TcStateToken, VerificationToken,
    };
    use crate::pus::{EcssSender, EcssTmtcSendError, PusTmWrapper};
    use crate::tmtc::tm_helper::SharedTmStore;
    use crate::SenderId;
    use alloc::vec::Vec;
    use spacepackets::ecss::{PusError, SerializablePusPacket};
    use spacepackets::tc::PusTc;
    use spacepackets::time::cds::TimeProvider;
    use spacepackets::time::{StdTimestampError, TimeWriter};
    use std::cell::RefCell;
    use std::format;
    use std::string::String;
    use std::sync::{mpsc, RwLockWriteGuard};
    use thiserror::Error;

    #[derive(Debug, Clone, Error)]
    pub enum EcssTmtcErrorWithSend {
        #[error("store locking error")]
        StoreLock,
        #[error("generic store error: {0}")]
        Store(#[from] StoreError),
        #[error("generic PUS error: {0}")]
        Pus(#[from] PusError),
        #[error("not able to send address {0}")]
        CantSendAddr(StoreAddr),
        #[error("generic channel send error: {0}")]
        Send(#[from] EcssTmtcSendError),
    }

    /// Generic trait for a user supplied sender object.
    ///
    /// This sender object is responsible for sending PUS telemetry to a TM sink.
    pub trait EcssTmSenderCore: EcssSender {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcErrorWithSend>;
    }

    /// Generic trait for a user supplied sender object.
    ///
    /// This sender object is responsible for sending PUS telecommands to a TC recipient. Each
    /// telecommand can optionally have a token which contains its verification state.
    pub trait EcssTcSenderCore: EcssSender {
        fn send_tc(
            &self,
            tc: PusTc,
            token: Option<TcStateToken>,
        ) -> Result<(), EcssTmtcErrorWithSend>;
    }

    #[derive(Clone)]
    pub struct MpscTmInStoreSender {
        id: SenderId,
        name: &'static str,
        store_helper: SharedPool,
        sender: mpsc::Sender<StoreAddr>,
        pub ignore_poison_errors: bool,
    }

    impl EcssSender for MpscTmInStoreSender {
        fn id(&self) -> SenderId {
            self.id
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl MpscTmInStoreSender {
        pub fn send_direct_tm(
            &self,
            tmtc: impl SerializablePusPacket,
        ) -> Result<(), EcssTmtcErrorWithSend> {
            let operation = |mut store: RwLockWriteGuard<ShareablePoolProvider>| {
                let (addr, slice) = store.free_element(tmtc.len_packed())?;
                tmtc.write_to_bytes(slice)?;
                self.sender.send(addr)?;
                Ok(())
            };
            match self.store_helper.write() {
                Ok(pool) => operation(pool),
                Err(e) => {
                    if self.ignore_poison_errors {
                        operation(e.into_inner())
                    } else {
                        Err(EcssTmtcErrorWithSend::Send(
                            EcssTmtcSendError::RxDisconnected,
                        ))
                    }
                }
            }
        }
    }

    impl EcssTmSenderCore for MpscTmInStoreSender {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcErrorWithSend> {
            match tm {
                PusTmWrapper::InStore(addr) => self.sender.send(addr).map_err(
                    EcssTmtcErrorWithSend::Send(EcssTmtcSendError::RxDisconnected),
                ),
                PusTmWrapper::Direct(tm) => self.send_direct_tm(tm),
            }
        }
    }

    impl MpscTmInStoreSender {
        pub fn new(
            id: SenderId,
            name: &'static str,
            store_helper: SharedPool,
            sender: mpsc::Sender<StoreAddr>,
        ) -> Self {
            Self {
                id,
                name,
                store_helper,
                sender,
                ignore_poison_errors: false,
            }
        }
    }

    #[derive(Debug, Clone, Error)]
    pub enum MpscTmAsVecSenderError {
        #[error("Generic PUS error: {0}")]
        Pus(#[from] PusError),
        #[error("MPSC channel send error: {0}")]
        Send(#[from] mpsc::SendError<Vec<u8>>),
        #[error("can not handle store addresses")]
        CantSendAddr(StoreAddr),
        #[error("RX handle has disconnected")]
        RxDisconnected,
    }

    /// This class can be used if frequent heap allocations during run-time are not an issue.
    /// PUS TM packets will be sent around as [Vec]s. Please note that the current implementation
    /// of this class can not deal with store addresses, so it is assumed that is is always
    /// going to be called with direct packets.
    #[derive(Clone)]
    pub struct MpscTmAsVecSender {
        id: SenderId,
        sender: mpsc::Sender<Vec<u8>>,
        name: &'static str,
    }

    impl MpscTmAsVecSender {
        pub fn new(id: u32, name: &'static str, sender: mpsc::Sender<Vec<u8>>) -> Self {
            Self { id, sender, name }
        }
    }

    impl EcssSender for MpscTmAsVecSender {
        fn id(&self) -> SenderId {
            self.id
        }
        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl EcssTmSenderCore for MpscTmAsVecSender {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcErrorWithSend> {
            match tm {
                PusTmWrapper::InStore(addr) => Err(EcssTmtcErrorWithSend::CantSendAddr(addr)),
                PusTmWrapper::Direct(tm) => {
                    let mut vec = Vec::new();
                    tm.append_to_vec(&mut vec)
                        .map_err(MpscTmAsVecSenderError::Pus)?;
                    self.sender
                        .send(vec)
                        .map_err(MpscTmAsVecSenderError::Send)?;
                    Ok(())
                }
            }
        }
    }

    #[derive(Debug, Clone, Error)]
    pub enum PusPacketHandlingError {
        #[error("Generic PUS error: {0}")]
        PusError(#[from] PusError),
        #[error("Wrong service number {0} for packet handler")]
        WrongService(u8),
        #[error("Invalid subservice {0}")]
        InvalidSubservice(u8),
        #[error("Not enough application data available: {0}")]
        NotEnoughAppData(String),
        #[error("Invalid application data")]
        InvalidAppData(String),
        #[error("Generic store error: {0}")]
        StoreError(#[from] StoreError),
        #[error("Error with the pool RwGuard: {0}")]
        RwGuardError(String),
        #[error("MQ send error: {0}")]
        SendError(String),
        #[error("TX message queue side has disconnected")]
        QueueDisconnected,
        #[error("Other error {0}")]
        OtherError(String),
    }

    #[derive(Debug, Clone, Error)]
    pub enum PartialPusHandlingError {
        #[error("Generic timestamp generation error")]
        Time(StdTimestampError),
        #[error("Error sending telemetry: {0}")]
        TmSend(String),
        #[error("Error sending verification message")]
        Verification,
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

    /// Generic abstraction for a telecommand being sent around after is has been accepted.
    /// The actual telecommand is stored inside a pre-allocated pool structure.
    pub type AcceptedTc = (StoreAddr, VerificationToken<TcStateAccepted>);

    /// Base class for handlers which can handle PUS TC packets. Right now, the message queue
    /// backend is constrained to [mpsc::channel]s and the verification reporter
    /// is constrained to the [StdVerifReporterWithSender].
    pub struct PusServiceBase {
        pub tc_rx: mpsc::Receiver<AcceptedTc>,
        pub tc_store: SharedPool,
        pub tm_tx: mpsc::Sender<StoreAddr>,
        pub tm_store: SharedTmStore,
        pub tm_apid: u16,
        /// The verification handler is wrapped in a [RefCell] to allow the interior mutability
        /// pattern. This makes writing methods which are not mutable a lot easier.
        pub verification_handler: RefCell<StdVerifReporterWithSender>,
        pub pus_buf: [u8; 2048],
        pub pus_size: usize,
    }

    impl PusServiceBase {
        pub fn new(
            receiver: mpsc::Receiver<AcceptedTc>,
            tc_pool: SharedPool,
            tm_tx: mpsc::Sender<StoreAddr>,
            tm_store: SharedTmStore,
            tm_apid: u16,
            verification_handler: StdVerifReporterWithSender,
        ) -> Self {
            Self {
                tc_rx: receiver,
                tc_store: tc_pool,
                tm_apid,
                tm_tx,
                tm_store,
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
                .tc_store
                .write()
                .map_err(|e| PusPacketHandlingError::RwGuardError(format!("{e}")))?;
            let tc_guard = tc_pool.read_with_guard(addr);
            let tc_raw = tc_guard.read().unwrap();
            psb_mut.pus_buf[0..tc_raw.len()].copy_from_slice(tc_raw);
            Ok(())
        }

        fn handle_next_packet(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
            return match self.psb().tc_rx.try_recv() {
                Ok((addr, token)) => self.handle_one_tc(addr, token),
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => Ok(PusPacketHandlerResult::Empty),
                    mpsc::TryRecvError::Disconnected => {
                        Err(PusPacketHandlingError::QueueDisconnected)
                    }
                },
            };
        }
    }
}

pub(crate) fn source_buffer_large_enough(
    cap: usize,
    len: usize,
) -> Result<(), EcssTmtcErrorWithSend> {
    if len > cap {
        return Err(EcssTmtcErrorWithSend::ByteConversion(
            ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                found: cap,
                expected: len,
            }),
        ));
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
