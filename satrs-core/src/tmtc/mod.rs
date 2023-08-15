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
use spacepackets::ecss::tc::PusTc;

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
