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
pub mod scheduler;
pub mod scheduler_srv;
#[cfg(feature = "std")]
pub mod test;
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
    use crate::pus::verification::{
        StdVerifReporterWithSender, TcStateAccepted, VerificationToken,
    };
    use crate::pus::{EcssSender, EcssTcSenderCore, EcssTmSenderCore};
    use crate::tmtc::tm_helper::SharedTmStore;
    use crate::SenderId;
    use alloc::vec::Vec;
    use spacepackets::ecss::{PusError, SerializablePusPacket};
    use spacepackets::tc::PusTc;
    use spacepackets::time::cds::TimeProvider;
    use spacepackets::time::{StdTimestampError, TimeWriter};
    use spacepackets::tm::PusTm;
    use std::string::String;
    use std::sync::{mpsc, RwLockWriteGuard};
    use thiserror::Error;

    #[derive(Debug, Clone, Error)]
    pub enum MpscPusInStoreSendError {
        #[error("RwGuard lock error")]
        LockError,
        #[error("Generic PUS error: {0}")]
        PusError(#[from] PusError),
        #[error("Generic store error: {0}")]
        StoreError(#[from] StoreError),
        #[error("Generic send error: {0}")]
        SendError(#[from] mpsc::SendError<StoreAddr>),
        #[error("RX handle has disconnected")]
        RxDisconnected(StoreAddr),
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

    impl MpscTmtcInStoreSender {
        pub fn send_tmtc(
            &mut self,
            tmtc: impl SerializablePusPacket,
        ) -> Result<(), MpscPusInStoreSendError> {
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
                        Err(MpscPusInStoreSendError::LockError)
                    }
                }
            }
        }
    }

    impl EcssTmSenderCore for MpscTmtcInStoreSender {
        type Error = MpscPusInStoreSendError;

        fn send_tm(&mut self, tm: PusTm) -> Result<(), Self::Error> {
            self.send_tmtc(tm)
        }
    }

    impl EcssTcSenderCore for MpscTmtcInStoreSender {
        type Error = MpscPusInStoreSendError;

        fn send_tc(&mut self, tc: PusTc) -> Result<(), Self::Error> {
            self.send_tmtc(tc)
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
        SendError(mpsc::SendError<Vec<u8>>),
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

    #[derive(Debug, Clone, Error)]
    pub enum PusPacketHandlingError {
        #[error("Generic PUS error: {0}")]
        PusError(#[from] PusError),
        #[error("Wrong service number {0} for packet handler")]
        WrongService(u8),
        #[error("Not enough application data available: {0}")]
        NotEnoughAppData(String),
        #[error("Generic store error: {0}")]
        StoreError(#[from] StoreError),
        #[error("Error with the pool RwGuard")]
        RwGuardError(String),
        #[error("MQ backend disconnect error")]
        QueueDisconnected,
        #[error("Other error {0}")]
        OtherError(String),
    }

    #[derive(Debug, Clone, Error)]
    pub enum PartialPusHandlingError {
        #[error("Generic timestamp generation error")]
        TimeError(StdTimestampError),
        #[error("Error sending telemetry: {0}")]
        TmSendError(String),
        #[error("Error sending verification message")]
        VerificationError,
    }

    #[derive(Debug, Clone)]
    pub enum PusPacketHandlerResult {
        RequestHandled,
        RequestHandledPartialSuccess(PartialPusHandlingError),
        CustomSubservice(VerificationToken<TcStateAccepted>),
        Empty,
    }

    impl From<PartialPusHandlingError> for PusPacketHandlerResult {
        fn from(value: PartialPusHandlingError) -> Self {
            Self::RequestHandledPartialSuccess(value)
        }
    }

    pub type AcceptedTc = (StoreAddr, VerificationToken<TcStateAccepted>);

    pub struct PusServiceBase {
        pub(crate) tc_rx: mpsc::Receiver<AcceptedTc>,
        pub(crate) tc_store: SharedPool,
        pub(crate) tm_tx: mpsc::Sender<StoreAddr>,
        pub(crate) tm_store: SharedTmStore,
        pub(crate) tm_apid: u16,
        pub(crate) verification_handler: StdVerifReporterWithSender,
        pub(crate) stamp_buf: [u8; 7],
        pub(crate) pus_buf: [u8; 2048],
        pus_size: usize,
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
                verification_handler,
                stamp_buf: [0; 7],
                pus_buf: [0; 2048],
                pus_size: 0,
            }
        }

        pub fn update_stamp(&mut self) -> Result<(), PartialPusHandlingError> {
            let time_provider =
                TimeProvider::from_now_with_u16_days().map_err(PartialPusHandlingError::TimeError);
            if let Ok(time_provider) = time_provider {
                time_provider.write_to_bytes(&mut self.stamp_buf).unwrap();
                Ok(())
            } else {
                self.stamp_buf = [0; 7];
                Err(time_provider.unwrap_err())
            }
        }
    }

    pub trait PusServiceHandler {
        fn psb_mut(&mut self) -> &mut PusServiceBase;
        fn psb(&self) -> &PusServiceBase;
        fn verification_reporter(&mut self) -> &mut StdVerifReporterWithSender {
            &mut self.psb_mut().verification_handler
        }
        fn tc_store(&mut self) -> &mut SharedPool {
            &mut self.psb_mut().tc_store
        }
        fn pus_tc_buf(&self) -> (&[u8], usize) {
            (&self.psb().pus_buf, self.psb().pus_size)
        }
        fn handle_one_tc(
            &mut self,
            addr: StoreAddr,
            token: VerificationToken<TcStateAccepted>,
        ) -> Result<PusPacketHandlerResult, PusPacketHandlingError>;
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
