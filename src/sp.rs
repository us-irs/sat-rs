//! # Space related components including CCSDS and ECSS packet standards
use ccsds_spacepacket::PrimaryHeader;
pub use ccsds_spacepacket::PrimaryHeader as DekuSpHeader;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq, Copy, Clone)]
pub enum PacketType {
    Tm = 0,
    Tc = 1,
}

impl TryFrom<u8> for PacketType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            x if x == PacketType::Tm as u8 => Ok(PacketType::Tm),
            x if x == PacketType::Tc as u8 => Ok(PacketType::Tc),
            _ => Err(()),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Copy, Clone)]
pub enum SequenceFlags {
    ContinuationSegment = 0b00,
    FirstSegment = 0b01,
    LastSegment = 0b10,
    Unsegmented = 0b11,
}

impl TryFrom<u8> for SequenceFlags {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            x if x == SequenceFlags::ContinuationSegment as u8 => {
                Ok(SequenceFlags::ContinuationSegment)
            }
            x if x == SequenceFlags::FirstSegment as u8 => Ok(SequenceFlags::FirstSegment),
            x if x == SequenceFlags::LastSegment as u8 => Ok(SequenceFlags::LastSegment),
            x if x == SequenceFlags::Unsegmented as u8 => Ok(SequenceFlags::Unsegmented),
            _ => Err(()),
        }
    }
}

/// Space Packet Primary Header according to CCSDS 133.0-B-2
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct SpHeader {
    pub version: u8,
    pub ptype: PacketType,
    pub apid: u16,
    pub secondary_header_flag: bool,
    pub sequence_flags: SequenceFlags,
    pub ssc: u16,
    pub packet_data_len: u16,
}

impl Default for SpHeader {
    fn default() -> Self {
        SpHeader {
            version: 0,
            ptype: PacketType::Tm,
            apid: 0,
            secondary_header_flag: true,
            sequence_flags: SequenceFlags::Unsegmented,
            ssc: 0,
            packet_data_len: 0,
        }
    }
}
impl SpHeader {
    pub fn new(apid: u16, ptype: PacketType, ssc: u16) -> Option<Self> {
        if ssc > num::pow(2, 14) || apid > num::pow(2, 11) {
            return None;
        }
        let mut header = SpHeader::default();
        header.ptype = ptype;
        header.apid = apid;
        header.ssc = ssc;
        Some(header)
    }

    pub fn tm(apid: u16, ssc: u16) -> Option<Self> {
        Self::new(apid, PacketType::Tm, ssc)
    }

    pub fn tc(apid: u16, ssc: u16) -> Option<Self> {
        Self::new(apid, PacketType::Tc, ssc)
    }

    /// Function to retrieve the packet sequence control field
    #[inline]
    pub fn psc(&self) -> u16 {
        ((self.sequence_flags as u16) << 14) | self.secondary_header_flag as u16
    }

    /// Retrieve Packet Identification composite field
    #[inline]
    pub fn packet_id(&self) -> u16 {
        ((self.ptype as u16) << 13) | ((self.secondary_header_flag as u16) << 12) | self.apid
    }

    #[inline]
    pub fn is_tm(&self) -> bool {
        self.ptype == PacketType::Tm
    }

    #[inline]
    pub fn is_tc(&self) -> bool {
        self.ptype == PacketType::Tc
    }
}

/// The [DekuSpHeader] is very useful to deserialize a packed raw space packet header with 6 bytes.
/// This function allows converting it to the [SpHeader] which is compatible to the [serde]
/// framework
impl TryFrom<DekuSpHeader> for SpHeader {
    type Error = ();

    fn try_from(header: PrimaryHeader) -> Result<Self, Self::Error> {
        let seq_num = SequenceFlags::try_from(header.sequence_flags as u8)?;
        let packet_type = PacketType::try_from(header.packet_type as u8)?;
        let sec_header_flag = header.sec_header_flag as u8 != 0;
        Ok(SpHeader {
            version: header.version,
            sequence_flags: seq_num,
            packet_data_len: header.data_length,
            ssc: header.sequence_count,
            ptype: packet_type,
            apid: header.app_proc_id,
            secondary_header_flag: sec_header_flag,
        })
    }
}

/// It is possible to convert the [serde] compatible [SpHeader] back into a [DekuSpHeader]
/// to allow for packed binary serialization
impl TryFrom<SpHeader> for DekuSpHeader {
    type Error = ();

    fn try_from(value: SpHeader) -> Result<Self, Self::Error> {
        use ccsds_spacepacket::types::PacketType as DekuPacketType;
        use ccsds_spacepacket::types::SecondaryHeaderFlag as DekuSecHeaderFlag;
        use ccsds_spacepacket::types::SeqFlag as DekuSeqFlag;
        let sequence_flags = match value.sequence_flags as u8 {
            x if x == SequenceFlags::Unsegmented as u8 => DekuSeqFlag::Unsegmented,
            x if x == SequenceFlags::FirstSegment as u8 => DekuSeqFlag::FirstSegment,
            x if x == SequenceFlags::LastSegment as u8 => DekuSeqFlag::LastSegment,
            x if x == SequenceFlags::ContinuationSegment as u8 => DekuSeqFlag::Continuation,
            _ => return Err(()),
        };
        let packet_type = match value.ptype as u8 {
            x if x == PacketType::Tm as u8 => DekuPacketType::Data,
            x if x == PacketType::Tc as u8 => DekuPacketType::Command,
            _ => return Err(()),
        };
        let sec_header_flag = match value.secondary_header_flag as bool {
            true => DekuSecHeaderFlag::Present,
            false => DekuSecHeaderFlag::NotPresent,
        };
        Ok(DekuSpHeader {
            version: value.version,
            packet_type,
            sec_header_flag,
            app_proc_id: value.apid,
            sequence_flags,
            data_length: value.packet_data_len,
            sequence_count: value.ssc,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::sp::{DekuSpHeader, PacketType, SequenceFlags, SpHeader};
    use deku::prelude::*;
    use postcard::{from_bytes, to_stdvec};

    #[test]
    fn test_deser_internally() {
        let sp_header = SpHeader::tc(0x42, 12).expect("Error creating SP header");
        assert_eq!(sp_header.version, 0b000);
        assert_eq!(sp_header.secondary_header_flag, true);
        assert_eq!(sp_header.ptype, PacketType::Tc);
        assert_eq!(sp_header.ssc, 12);
        assert_eq!(sp_header.apid, 0x42);
        assert_eq!(sp_header.sequence_flags, SequenceFlags::Unsegmented);
        assert_eq!(sp_header.packet_data_len, 0);
        let output = to_stdvec(&sp_header).unwrap();
        println!("Output: {:?} with length {}", output, output.len());
        let sp_header: SpHeader = from_bytes(&output).unwrap();
        assert_eq!(sp_header.version, 0b000);
        assert_eq!(sp_header.secondary_header_flag, true);
        assert_eq!(sp_header.ptype, PacketType::Tc);
        assert_eq!(sp_header.ssc, 12);
        assert_eq!(sp_header.apid, 0x42);
        assert_eq!(sp_header.sequence_flags, SequenceFlags::Unsegmented);
        assert_eq!(sp_header.packet_data_len, 0);
    }

    #[test]
    fn test_deser_to_raw_packed_deku() {
        let sp_header = SpHeader::tc(0x42, 12).expect("Error creating SP header");
        // TODO: Wait with these tests until KubOS merged
        //       https://github.com/KubOS-Preservation-Group/ccsds-spacepacket/pull/14
        let _deku_header =
            DekuSpHeader::try_from(sp_header).expect("Error creating Deku Sp Header");
        // deku_header.to_bytes().unwrap();
    }
}
