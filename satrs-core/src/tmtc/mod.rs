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
use spacepackets::{ByteConversionError, SizeMissmatch, SpHeader};

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
            return Err(ByteConversionError::FromSliceTooSmall(SizeMissmatch {
                found: buf.len(),
                expected: 8,
            }));
        }
        Ok(Self {
            target_id: u32::from_be_bytes(buf[0..4].try_into().unwrap()),
            unique_id: u32::from_be_bytes(buf[4..8].try_into().unwrap()),
        })
    }

    pub fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError> {
        if buf.len() < 8 {
            return Err(ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                found: buf.len(),
                expected: 8,
            }));
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
pub trait ReceivesTc: ReceivesTcCore + Downcast + Send {}

/// Blanket implementation to automatically implement [ReceivesTc] when the [alloc] feature
/// is enabled.
#[cfg(feature = "alloc")]
impl<T> ReceivesTc for T where T: ReceivesTcCore + Send + 'static {}

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
