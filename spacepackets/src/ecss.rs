use crate::CcsdsPacket;
use crc::{Crc, CRC_16_IBM_3740};
use serde::{Deserialize, Serialize};

/// CRC algorithm used by the PUS standard
pub const CRC_CCITT_FALSE: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_3740);

/// All PUS versions. Only PUS C is supported by this library
#[derive(PartialEq, Copy, Clone, Serialize, Deserialize, Debug)]
pub enum PusVersion {
    EsaPus = 0,
    PusA = 1,
    PusC = 2,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum PusError {
    VersionNotSupported(PusVersion),
}

pub trait PusPacket: CcsdsPacket {
    fn service(&self) -> u8;
    fn subservice(&self) -> u8;
    fn source_id(&self) -> u16;
    fn ack_flags(&self) -> u8;

    fn user_data(&self) -> Option<&[u8]>;
    fn crc16(&self) -> Option<u16>;
    /// Verify that the packet is valid. PUS packets have a CRC16 checksum to do this
    fn verify(&mut self) -> bool;
}
