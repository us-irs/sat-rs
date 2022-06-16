use crate::CcsdsPacket;

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
