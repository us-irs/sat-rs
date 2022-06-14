//! # Space related components including CCSDS and ECSS packet standards
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

/// Generic trait to access fields of a CCSDS space packet header according to CCSDS 133.0-B-2
pub trait CcsdsPrimaryHeader {
    const SEQ_FLAG_MASK: u16 = 0xC000;

    fn version(&self) -> u8;
    /// Retrieve 13 bit Packet Identification field. Can usually be retrieved with a bitwise AND
    /// of the first 2 bytes with 0x1FFF
    fn packet_id(&self) -> u16;
    /// Retrieve Packet Sequence Count
    fn psc(&self) -> u16;
    /// Retrieve data length field
    fn data_len(&self) -> u16;

    #[inline]
    /// Retrieve Packet Type (TM: 0, TC: 1)
    fn ptype(&self) -> PacketType {
        // This call should never fail because only 0 and 1 can be passed to the try_from call
        PacketType::try_from((self.packet_id() >> 12) as u8 & 0b1).unwrap()
    }
    #[inline]
    fn is_tm(&self) -> bool {
        self.ptype() == PacketType::Tm
    }

    #[inline]
    fn is_tc(&self) -> bool {
        self.ptype() == PacketType::Tc
    }

    /// Retrieve the secondary header flag. Returns true if a secondary header is present
    /// and false if it is not
    #[inline]
    fn sec_header_flag(&self) -> bool {
        (self.packet_id() >> 11) & 0x01 != 0
    }

    /// Retrieve Application Process ID
    #[inline]
    fn apid(&self) -> u16 {
        self.packet_id() & 0x7FF
    }

    #[inline]
    fn ssc(&self) -> u16 {
        self.psc() & (!Self::SEQ_FLAG_MASK)
    }

    #[inline]
    fn sequence_flags(&self) -> SequenceFlags {
        // This call should never fail because the mask ensures that only valid values are passed
        // into the try_from function
        SequenceFlags::try_from(((self.psc() & Self::SEQ_FLAG_MASK) >> 14) as u8).unwrap()
    }
}

pub mod srd {
    use crate::sp::SequenceFlags;
    use crate::sp::{CcsdsPrimaryHeader, PacketType};

    /// Space Packet Primary Header according to CCSDS 133.0-B-2
    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    pub struct SpHeader {
        pub version: u8,
        pub ptype: PacketType,
        pub apid: u16,
        pub sec_header_flag: bool,
        pub seq_flags: SequenceFlags,
        pub ssc: u16,
        pub data_len: u16,
    }
    impl Default for SpHeader {
        fn default() -> Self {
            SpHeader {
                version: 0,
                ptype: PacketType::Tm,
                apid: 0,
                sec_header_flag: true,
                seq_flags: SequenceFlags::Unsegmented,
                ssc: 0,
                data_len: 0,
            }
        }
    }
    impl SpHeader {
        pub fn new(apid: u16, ptype: PacketType, ssc: u16) -> Option<Self> {
            if ssc > num::pow(2, 14) || apid > num::pow(2, 11) {
                return None;
            }
            Some(SpHeader {
                ptype,
                apid,
                ssc,
                ..Default::default()
            })
        }

        pub fn tm(apid: u16, ssc: u16) -> Option<Self> {
            Self::new(apid, PacketType::Tm, ssc)
        }

        pub fn tc(apid: u16, ssc: u16) -> Option<Self> {
            Self::new(apid, PacketType::Tc, ssc)
        }
    }

    impl CcsdsPrimaryHeader for SpHeader {
        #[inline]
        fn version(&self) -> u8 {
            self.version
        }

        /// Retrieve Packet Identification composite field
        #[inline]
        fn packet_id(&self) -> u16 {
            ((self.ptype as u16) << 13) | ((self.sec_header_flag as u16) << 12) | self.apid
        }

        /// Function to retrieve the packet sequence control field
        #[inline]
        fn psc(&self) -> u16 {
            ((self.seq_flags as u16) << 14) | self.sec_header_flag as u16
        }

        #[inline]
        fn data_len(&self) -> u16 {
            self.data_len
        }
    }
}

pub mod zc {
    use crate::sp::CcsdsPrimaryHeader;
    use zerocopy::byteorder::NetworkEndian;
    use zerocopy::{AsBytes, FromBytes, Unaligned, U16};

    #[derive(FromBytes, AsBytes, Unaligned)]
    #[repr(C)]
    struct SpHeader {
        version_packet_id: U16<NetworkEndian>,
        psc: U16<NetworkEndian>,
        data_len: U16<NetworkEndian>,
    }

    impl CcsdsPrimaryHeader for SpHeader {
        #[inline]
        fn version(&self) -> u8 {
            ((self.version_packet_id.get() >> 13) as u8) & 0b111
        }

        #[inline]
        fn packet_id(&self) -> u16 {
            self.version_packet_id.get() & 0x1FFF
        }

        #[inline]
        fn psc(&self) -> u16 {
            self.psc.get()
        }

        #[inline]
        fn data_len(&self) -> u16 {
            self.data_len.get()
        }
    }
}

pub mod deku {
    use crate::sp::srd::SpHeader;
    use crate::sp::{CcsdsPrimaryHeader, DekuSpHeader, PacketType, SequenceFlags};
    use ccsds_spacepacket::PrimaryHeader;

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
                seq_flags: seq_num,
                data_len: header.data_length,
                ssc: header.sequence_count,
                ptype: packet_type,
                apid: header.app_proc_id,
                sec_header_flag,
            })
        }
    }

    impl CcsdsPrimaryHeader for DekuSpHeader {
        #[inline]
        fn version(&self) -> u8 {
            self.version
        }

        #[inline]
        fn packet_id(&self) -> u16 {
            ((self.packet_type as u16) << 12)
                | ((self.sec_header_flag as u16) << 11)
                | self.app_proc_id
        }

        #[inline]
        fn psc(&self) -> u16 {
            ((self.sequence_flags as u16) << 14) | self.sequence_count
        }

        #[inline]
        fn data_len(&self) -> u16 {
            self.data_length
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
            let sequence_flags = match value.seq_flags as u8 {
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
            let sec_header_flag = match value.sec_header_flag as bool {
                true => DekuSecHeaderFlag::Present,
                false => DekuSecHeaderFlag::NotPresent,
            };
            Ok(DekuSpHeader {
                version: value.version,
                packet_type,
                sec_header_flag,
                app_proc_id: value.apid,
                sequence_flags,
                data_length: value.data_len,
                sequence_count: value.ssc,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::sp::srd::SpHeader;
    use crate::sp::{DekuSpHeader, PacketType, SequenceFlags};
    use postcard::{from_bytes, to_stdvec};

    #[test]
    fn test_deser_internally() {
        let sp_header = SpHeader::tc(0x42, 12).expect("Error creating SP header");
        assert_eq!(sp_header.version, 0b000);
        assert_eq!(sp_header.sec_header_flag, true);
        assert_eq!(sp_header.ptype, PacketType::Tc);
        assert_eq!(sp_header.ssc, 12);
        assert_eq!(sp_header.apid, 0x42);
        assert_eq!(sp_header.seq_flags, SequenceFlags::Unsegmented);
        assert_eq!(sp_header.data_len, 0);
        let output = to_stdvec(&sp_header).unwrap();
        println!("Output: {:?} with length {}", output, output.len());
        let sp_header: SpHeader = from_bytes(&output).unwrap();
        assert_eq!(sp_header.version, 0b000);
        assert_eq!(sp_header.sec_header_flag, true);
        assert_eq!(sp_header.ptype, PacketType::Tc);
        assert_eq!(sp_header.ssc, 12);
        assert_eq!(sp_header.apid, 0x42);
        assert_eq!(sp_header.seq_flags, SequenceFlags::Unsegmented);
        assert_eq!(sp_header.data_len, 0);
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
