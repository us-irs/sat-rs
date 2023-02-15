//! # PUS support modules
#[cfg(feature = "alloc")]
use downcast_rs::{impl_downcast, Downcast};
#[cfg(feature = "alloc")]
use dyn_clone::DynClone;
use spacepackets::ecss::PusError;
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

#[derive(Debug, Clone)]
pub enum EcssTmErrorWithSend<E> {
    /// Errors related to sending the verification telemetry to a TM recipient
    SendError(E),
    EcssTmError(EcssTmError),
}

impl<E> From<EcssTmError> for EcssTmErrorWithSend<E> {
    fn from(value: EcssTmError) -> Self {
        Self::EcssTmError(value)
    }
}

/// Generic error type which is also able to wrap a user send error with the user supplied type E.
#[derive(Debug, Clone)]
pub enum EcssTmError {
    /// Errors related to the time stamp format of the telemetry
    TimestampError(TimestampError),
    /// Errors related to byte conversion, for example insufficient buffer size for given data
    ByteConversionError(ByteConversionError),
    /// Errors related to PUS packet format
    PusError(PusError),
}

impl From<PusError> for EcssTmError {
    fn from(e: PusError) -> Self {
        EcssTmError::PusError(e)
    }
}

impl From<ByteConversionError> for EcssTmError {
    fn from(e: ByteConversionError) -> Self {
        EcssTmError::ByteConversionError(e)
    }
}

/// Generic trait for a user supplied sender object.
///
/// This sender object is responsible for sending telemetry to a TM sink.
pub trait EcssTmSenderCore: Send {
    type Error;

    fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmErrorWithSend<Self::Error>>;
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
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum GenericTcCheckError {
    NotEnoughAppData,
    InvalidSubservice,
}

pub(crate) fn source_buffer_large_enough(cap: usize, len: usize) -> Result<(), EcssTmError> {
    if len > cap {
        return Err(EcssTmError::ByteConversionError(
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
