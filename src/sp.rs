//! # Space related components including CCSDS and ECSS packet standards
use serde::{Serialize, Deserialize};

pub enum PacketType {
    Tm = 0,
    Tc = 1
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
struct SpHeader {
    pub version: u8,
    pub ptype: PacketType,
    pub apid: u16,
    pub secondary_header_flag: u8,
    pub ssc: u16,
}