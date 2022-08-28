//! Telemetry and Telecommanding (TMTC) module. Contains packet routing components with special
//! support for CCSDS and ECSS packets.
//!
//! The distributor modules provided by this module use trait objects provided by the user to
//! directly dispatch received packets to packet listeners based on packet fields like the CCSDS
//! Application Process ID (APID) or the ECSS PUS service type. This allows for fast packet
//! routing without the overhead and complication of using message queues. However, it also requires
use crate::error::{FsrcErrorRaw, FsrcGroupIds};
use spacepackets::tc::PusTc;
use spacepackets::SpHeader;

#[cfg(feature = "alloc")]
pub mod ccsds_distrib;
#[cfg(feature = "alloc")]
pub mod pus_distrib;

pub use ccsds_distrib::{CcsdsDistributor, CcsdsError, CcsdsPacketHandler};
pub use pus_distrib::{PusDistributor, PusServiceProvider};

const _RAW_PACKET_ERROR: &str = "raw-tmtc";
const _CCSDS_ERROR: &str = "ccsds-tmtc";
const _PUS_ERROR: &str = "pus-tmtc";

// TODO: A macro for general and unknown errors would be nice
const _FROM_BYTES_SLICE_TOO_SMALL_ERROR: FsrcErrorRaw = FsrcErrorRaw::new(
    FsrcGroupIds::Tmtc as u8,
    0,
    _RAW_PACKET_ERROR,
    "FROM_BYTES_SLICE_TOO_SMALL_ERROR",
);

const _FROM_BYTES_ZEROCOPY_ERROR: FsrcErrorRaw = FsrcErrorRaw::new(
    FsrcGroupIds::Tmtc as u8,
    1,
    _RAW_PACKET_ERROR,
    "FROM_BYTES_ZEROCOPY_ERROR",
);

/// Generic trait for object which can receive any telecommands in form of a raw bytestream, with
/// no assumptions about the received protocol.
///
/// This trait is implemented by both the [crate::tmtc::pus_distrib::PusDistributor] and the
/// [crate::tmtc::ccsds_distrib::CcsdsDistributor]  which allows to pass the respective packets in
/// raw byte format into them.
pub trait ReceivesTc {
    type Error;
    fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error>;
}

/// Generic trait for object which can receive CCSDS space packets, for fsrc-example ECSS PUS packets
/// for CCSDS File Delivery Protocol (CFDP) packets.
///
/// This trait is implemented by both the [crate::tmtc::pus_distrib::PusDistributor] and the
/// [crate::tmtc::ccsds_distrib::CcsdsDistributor] which allows
/// to pass the respective packets in raw byte format or in CCSDS format into them.
pub trait ReceivesCcsdsTc {
    type Error;
    fn pass_ccsds(&mut self, header: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error>;
}

/// Generic trait for objects which can receive ECSS PUS telecommands. This trait is
/// implemented by the [crate::tmtc::pus_distrib::PusDistributor] objects to allow passing PUS TC
/// packets into it.
pub trait ReceivesEcssPusTc {
    type Error;
    fn pass_pus_tc(&mut self, header: &SpHeader, pus_tc: &PusTc) -> Result<(), Self::Error>;
}
