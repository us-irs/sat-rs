//! Telemetry and Telecommanding (TMTC) module. Contains packet routing components with special
//! support for CCSDS and ECSS packets.
//!
//! The distributor modules provided by this module use trait objects provided by the user to
//! directly dispatch received packets to packet listeners based on packet fields like the CCSDS
//! Application Process ID (APID) or the ECSS PUS service type. This allows for fast packet
//! routing without the overhead and complication of using message queues. However, it also requires
#[cfg(feature = "alloc")]
use downcast_rs::{impl_downcast, Downcast};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use spacepackets::{ByteConversionError, SpHeader};

#[cfg(feature = "alloc")]
pub mod ccsds_distrib;
#[cfg(feature = "alloc")]
pub mod pus_distrib;
pub mod tm_helper;

#[cfg(feature = "alloc")]
pub use ccsds_distrib::{CcsdsDistributor, CcsdsError, CcsdsPacketHandler};
#[cfg(feature = "alloc")]
pub use pus_distrib::{PusDistributor, PusServiceProvider};

pub type TargetId = u32;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AddressableId {
    pub target_id: TargetId,
    pub unique_id: u32,
}

impl AddressableId {
    pub fn from_raw_be(buf: &[u8]) -> Result<Self, ByteConversionError> {
        if buf.len() < 8 {
            return Err(ByteConversionError::FromSliceTooSmall {
                found: buf.len(),
                expected: 8,
            });
        }
        Ok(Self {
            target_id: u32::from_be_bytes(buf[0..4].try_into().unwrap()),
            unique_id: u32::from_be_bytes(buf[4..8].try_into().unwrap()),
        })
    }

    pub fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError> {
        if buf.len() < 8 {
            return Err(ByteConversionError::ToSliceTooSmall {
                found: buf.len(),
                expected: 8,
            });
        }
        buf[0..4].copy_from_slice(&self.target_id.to_be_bytes());
        buf[4..8].copy_from_slice(&self.unique_id.to_be_bytes());
        Ok(8)
    }
}

/// Generic trait for object which can receive any telecommands in form of a raw bytestream, with
/// no assumptions about the received protocol.
///
/// This trait is implemented by both the [crate::tmtc::pus_distrib::PusDistributor] and the
/// [crate::tmtc::ccsds_distrib::CcsdsDistributor]  which allows to pass the respective packets in
/// raw byte format into them.
pub trait ReceivesTcCore {
    type Error;
    fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error>;
}

/// Extension trait of [ReceivesTcCore] which allows downcasting by implementing [Downcast] and
/// is also sendable.
#[cfg(feature = "alloc")]
pub trait ReceivesTc: ReceivesTcCore + Downcast + Send {
    // Remove this once trait upcasting coercion has been implemented.
    // Tracking issue: https://github.com/rust-lang/rust/issues/65991
    fn upcast(&self) -> &dyn ReceivesTcCore<Error = Self::Error>;
    // Remove this once trait upcasting coercion has been implemented.
    // Tracking issue: https://github.com/rust-lang/rust/issues/65991
    fn upcast_mut(&mut self) -> &mut dyn ReceivesTcCore<Error = Self::Error>;
}

/// Blanket implementation to automatically implement [ReceivesTc] when the [alloc] feature
/// is enabled.
#[cfg(feature = "alloc")]
impl<T> ReceivesTc for T
where
    T: ReceivesTcCore + Send + 'static,
{
    // Remove this once trait upcasting coercion has been implemented.
    // Tracking issue: https://github.com/rust-lang/rust/issues/65991
    fn upcast(&self) -> &dyn ReceivesTcCore<Error = Self::Error> {
        self
    }
    // Remove this once trait upcasting coercion has been implemented.
    // Tracking issue: https://github.com/rust-lang/rust/issues/65991
    fn upcast_mut(&mut self) -> &mut dyn ReceivesTcCore<Error = Self::Error> {
        self
    }
}

#[cfg(feature = "alloc")]
impl_downcast!(ReceivesTc assoc Error);

/// Generic trait for object which can receive CCSDS space packets, for example ECSS PUS packets
/// for CCSDS File Delivery Protocol (CFDP) packets.
///
/// This trait is implemented by both the [crate::tmtc::pus_distrib::PusDistributor] and the
/// [crate::tmtc::ccsds_distrib::CcsdsDistributor] which allows
/// to pass the respective packets in raw byte format or in CCSDS format into them.
pub trait ReceivesCcsdsTc {
    type Error;
    fn pass_ccsds(&mut self, header: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error>;
}

/// Generic trait for a TM packet source, with no restrictions on the type of TM.
/// Implementors write the telemetry into the provided buffer and return the size of the telemetry.
pub trait TmPacketSourceCore {
    type Error;
    fn retrieve_packet(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error>;
}

/// Extension trait of [TmPacketSourceCore] which allows downcasting by implementing [Downcast] and
/// is also sendable.
#[cfg(feature = "alloc")]
pub trait TmPacketSource: TmPacketSourceCore + Downcast + Send {
    // Remove this once trait upcasting coercion has been implemented.
    // Tracking issue: https://github.com/rust-lang/rust/issues/65991
    fn upcast(&self) -> &dyn TmPacketSourceCore<Error = Self::Error>;
    // Remove this once trait upcasting coercion has been implemented.
    // Tracking issue: https://github.com/rust-lang/rust/issues/65991
    fn upcast_mut(&mut self) -> &mut dyn TmPacketSourceCore<Error = Self::Error>;
}

/// Blanket implementation to automatically implement [ReceivesTc] when the [alloc] feature
/// is enabled.
#[cfg(feature = "alloc")]
impl<T> TmPacketSource for T
where
    T: TmPacketSourceCore + Send + 'static,
{
    // Remove this once trait upcasting coercion has been implemented.
    // Tracking issue: https://github.com/rust-lang/rust/issues/65991
    fn upcast(&self) -> &dyn TmPacketSourceCore<Error = Self::Error> {
        self
    }
    // Remove this once trait upcasting coercion has been implemented.
    // Tracking issue: https://github.com/rust-lang/rust/issues/65991
    fn upcast_mut(&mut self) -> &mut dyn TmPacketSourceCore<Error = Self::Error> {
        self
    }
}
