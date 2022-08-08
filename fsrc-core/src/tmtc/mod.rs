use crate::error::{FsrcErrorRaw, FsrcGroupIds};
use spacepackets::ecss::PusError;
use spacepackets::tc::PusTc;
use spacepackets::{PacketError, SpHeader};

pub mod ccsds_distrib;
pub mod pus_distrib;

const RAW_PACKET_ERROR: &str = "tmtc-raw";
const CCSDS_ERROR: &str = "tmtc-ccsds";
const PUS_ERROR: &str = "tmtc-pus";

// TODO: A macro for general and unknown errors would be nice
const FROM_BYTES_SLICE_TOO_SMALL_ERROR: FsrcErrorRaw = FsrcErrorRaw {
    group_name: RAW_PACKET_ERROR,
    group_id: FsrcGroupIds::Tmtc as u8,
    unique_id: 0,
    error_info: "FROM_BYTES_SLICE_TOO_SMALL_ERROR",
};
const FROM_BYTES_ZEROCOPY_ERROR: FsrcErrorRaw = FsrcErrorRaw {
    group_name: RAW_PACKET_ERROR,
    group_id: FsrcGroupIds::Tmtc as u8,
    unique_id: 1,
    error_info: "FROM_BYTES_ZEROCOPY_ERROR",
};

pub trait ReceivesTc {
    fn pass_tc(&mut self, tc_raw: &[u8]);
}

pub trait ReceivesCcsds {
    fn pass_ccsds(&mut self, header: &SpHeader, tm_raw: &[u8]) -> Result<(), PacketError>;
}

pub trait ReceivesPus {
    fn pass_pus(&mut self, pus_tc: &PusTc) -> Result<(), PusError>;
}
