use spacepackets::ecss::PusError;
use spacepackets::tc::PusTc;
use spacepackets::{PacketError, SpHeader};

pub mod ccsds_distrib;
pub mod pus_distrib;

pub trait ReceivesTc {
    fn pass_tc(&mut self, tc_raw: &[u8]);
}

pub trait ReceivesCcsds {
    fn pass_ccsds(&mut self, header: &SpHeader, tm_raw: &[u8]) -> Result<(), PacketError>;
}

pub trait ReceivesPus {
    fn pass_pus(&mut self, pus_tc: &PusTc) -> Result<(), PusError>;
}
