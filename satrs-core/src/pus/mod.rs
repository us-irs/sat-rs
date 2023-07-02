//! # PUS support modules
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
pub mod hk;
pub mod mode;
#[cfg(feature = "std")]
pub mod scheduling;
pub mod verification;

#[cfg(feature = "alloc")]
pub use alloc_mod::*;

use crate::SenderId;
#[cfg(feature = "std")]
pub use std_mod::*;

#[derive(Debug, Clone)]
pub enum EcssTmtcErrorWithSend<E> {
    /// Errors related to sending the telemetry to a TMTC recipient
    SendError(E),
    EcssTmtcError(EcssTmtcError),
}

impl<E> From<EcssTmtcError> for EcssTmtcErrorWithSend<E> {
    fn from(value: EcssTmtcError) -> Self {
        Self::EcssTmtcError(value)
    }
}

/// Generic error type for PUS TM handling.
#[derive(Debug, Clone)]
pub enum EcssTmtcError {
    /// Errors related to the time stamp format of the telemetry
    TimestampError(TimestampError),
    /// Errors related to byte conversion, for example insufficient buffer size for given data
    ByteConversionError(ByteConversionError),
    /// Errors related to PUS packet format
    PusError(PusError),
}

impl From<PusError> for EcssTmtcError {
    fn from(e: PusError) -> Self {
        EcssTmtcError::PusError(e)
    }
}

impl From<ByteConversionError> for EcssTmtcError {
    fn from(e: ByteConversionError) -> Self {
        EcssTmtcError::ByteConversionError(e)
    }
}

pub trait EcssSender: Send {
    /// Each sender can have an ID associated with it
    fn id(&self) -> SenderId;
    fn name(&self) -> &'static str {
        "unset"
    }
}
/// Generic trait for a user supplied sender object.
///
/// This sender object is responsible for sending PUS telemetry to a TM sink.
pub trait EcssTmSenderCore: EcssSender {
    type Error;

    fn send_tm(&mut self, tm: PusTm) -> Result<(), Self::Error>;
}

/// Generic trait for a user supplied sender object.
///
/// This sender object is responsible for sending PUS telecommands to a TC recipient.
pub trait EcssTcSenderCore: EcssSender {
    type Error;

    fn send_tc(&mut self, tc: PusTc) -> Result<(), Self::Error>;
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

    dyn_clone::clone_trait_object!(<T> EcssTmSender<Error=T>);
    impl_downcast!(EcssTmSender assoc Error);

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

    dyn_clone::clone_trait_object!(<T> EcssTcSender<Error=T>);
    impl_downcast!(EcssTcSender assoc Error);
}

#[cfg(feature = "std")]
pub mod std_mod {
    use crate::pool::{ShareablePoolProvider, SharedPool, StoreAddr, StoreError};
    use crate::pus::{EcssSender, EcssTcSenderCore, EcssTmSenderCore};
    use crate::SenderId;
    use alloc::vec::Vec;
    use spacepackets::ecss::{PusError, SerializablePusPacket};
    use spacepackets::tc::PusTc;
    use spacepackets::tm::PusTm;
    use std::sync::mpsc::SendError;
    use std::sync::{mpsc, RwLockWriteGuard};

    #[derive(Debug, Clone)]
    pub enum MpscPusInStoreSendError {
        LockError,
        PusError(PusError),
        StoreError(StoreError),
        SendError(SendError<StoreAddr>),
        RxDisconnected(StoreAddr),
    }

    impl From<PusError> for MpscPusInStoreSendError {
        fn from(value: PusError) -> Self {
            MpscPusInStoreSendError::PusError(value)
        }
    }
    impl From<SendError<StoreAddr>> for MpscPusInStoreSendError {
        fn from(value: SendError<StoreAddr>) -> Self {
            MpscPusInStoreSendError::SendError(value)
        }
    }
    impl From<StoreError> for MpscPusInStoreSendError {
        fn from(value: StoreError) -> Self {
            MpscPusInStoreSendError::StoreError(value)
        }
    }

    #[derive(Clone)]
    pub struct MpscTmtcInStoreSender {
        id: SenderId,
        name: &'static str,
        store_helper: SharedPool,
        sender: mpsc::Sender<StoreAddr>,
        pub ignore_poison_errors: bool,
    }

    impl EcssSender for MpscTmtcInStoreSender {
        fn id(&self) -> SenderId {
            self.id
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl EcssTmSenderCore for MpscTmtcInStoreSender {
        type Error = MpscPusInStoreSendError;

        fn send_tm(&mut self, tm: PusTm) -> Result<(), Self::Error> {
            let operation = |mut store: RwLockWriteGuard<ShareablePoolProvider>| {
                let (addr, slice) = store.free_element(tm.len_packed())?;
                tm.write_to_bytes(slice)?;
                self.sender.send(addr)?;
                Ok(())
            };
            match self.store_helper.write() {
                Ok(pool) => operation(pool),
                Err(e) => {
                    if self.ignore_poison_errors {
                        operation(e.into_inner())
                    } else {
                        Err(MpscPusInStoreSendError::LockError)
                    }
                }
            }
        }
    }

    impl EcssTcSenderCore for MpscTmtcInStoreSender {
        type Error = MpscPusInStoreSendError;

        fn send_tc(&mut self, tc: PusTc) -> Result<(), Self::Error> {
            let operation = |mut store: RwLockWriteGuard<ShareablePoolProvider>| {
                let (addr, slice) = store.free_element(tc.len_packed())?;
                tc.write_to_bytes(slice)?;
                self.sender.send(addr)?;
                Ok(())
            };
            match self.store_helper.write() {
                Ok(pool) => operation(pool),
                Err(e) => {
                    if self.ignore_poison_errors {
                        operation(e.into_inner())
                    } else {
                        Err(MpscPusInStoreSendError::LockError)
                    }
                }
            }
        }
    }

    impl MpscTmtcInStoreSender {
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

    #[derive(Debug, Clone)]
    pub enum MpscAsVecSenderError {
        PusError(PusError),
        SendError(SendError<Vec<u8>>),
    }

    #[derive(Debug, Clone)]
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
        type Error = MpscAsVecSenderError;

        fn send_tm(&mut self, tm: PusTm) -> Result<(), Self::Error> {
            let mut vec = Vec::new();
            tm.append_to_vec(&mut vec)
                .map_err(MpscAsVecSenderError::PusError)?;
            self.sender
                .send(vec)
                .map_err(MpscAsVecSenderError::SendError)?;
            Ok(())
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GenericTcCheckError {
    NotEnoughAppData,
    InvalidSubservice,
}

pub(crate) fn source_buffer_large_enough(cap: usize, len: usize) -> Result<(), EcssTmtcError> {
    if len > cap {
        return Err(EcssTmtcError::ByteConversionError(
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
