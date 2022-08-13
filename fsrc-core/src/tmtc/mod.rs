use crate::error::{FsrcErrorRaw, FsrcGroupIds};
use spacepackets::{PacketError, SpHeader};

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
    fn pass_tc(&mut self, tc_raw: &[u8]);
}

pub trait ReceivesCcsdsTc {
    fn pass_ccsds(&mut self, header: &SpHeader, tc_raw: &[u8]) -> Result<(), PacketError>;
}
