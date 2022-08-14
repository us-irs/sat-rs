//! TMTC module. Contains packet routing components with special support for CCSDS and ECSS packets.
use crate::error::{FsrcErrorRaw, FsrcGroupIds};
use spacepackets::tc::PusTc;
use spacepackets::SpHeader;

pub mod ccsds_distrib;
pub mod pus_distrib;

const RAW_PACKET_ERROR: &str = "raw-tmtc";
const _CCSDS_ERROR: &str = "ccsds-tmtc";
const _PUS_ERROR: &str = "pus-tmtc";

// TODO: A macro for general and unknown errors would be nice
const FROM_BYTES_SLICE_TOO_SMALL_ERROR: FsrcErrorRaw = FsrcErrorRaw::new(
    FsrcGroupIds::Tmtc as u8,
    0,
    RAW_PACKET_ERROR,
    "FROM_BYTES_SLICE_TOO_SMALL_ERROR",
);

const FROM_BYTES_ZEROCOPY_ERROR: FsrcErrorRaw = FsrcErrorRaw::new(
    FsrcGroupIds::Tmtc as u8,
    1,
    RAW_PACKET_ERROR,
    "FROM_BYTES_ZEROCOPY_ERROR",
);

pub trait ReceivesTc {
    type Error;
    // TODO: Maybe it makes sense to return Result<(), Self::Error> here with Error being an associated
    //       type..
    fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error>;
}

pub trait ReceivesCcsdsTc {
    type Error;
    fn pass_ccsds(&mut self, header: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error>;
}

pub trait ReceivesEcssPusTc {
    type Error;
    fn pass_pus_tc(&mut self, header: &SpHeader, pus_tc: &PusTc) -> Result<(), Self::Error>;
}
