
/*
pub mod deku {
    pub use ccsds_spacepacket::PrimaryHeader as SpHeader;
    use crate::sp::{self, PacketId, PacketSequenceCtrl};
    use crate::sp::{CcsdsPrimaryHeader, PacketType, SequenceFlags};

    impl CcsdsPrimaryHeader for SpHeader {
        fn from_composite_fields(packet_id: PacketId, psc: PacketSequenceCtrl, data_len: u16, version: Option<u8>) -> Self {
            let mut version_to_set = 0b000;
            if let Some(version) = version {
                version_to_set = version;
            }
            let packet_type = match packet_id.ptype {
                PacketType::Tm => ccsds_spacepacket::types::PacketType::Data,
                PacketType::Tc => ccsds_spacepacket::types::PacketType::Command
            };
            let sec_header_flag = match packet_id.sec_header_flag {
                true => ccsds_spacepacket::types::SecondaryHeaderFlag::Present,
                false => ccsds_spacepacket::types::SecondaryHeaderFlag::NotPresent
            };
            let sequence_flags = match psc.seq_flags {
                SequenceFlags::ContinuationSegment => ccsds_spacepacket::types::SeqFlag::Continuation,
                SequenceFlags::FirstSegment => ccsds_spacepacket::types::SeqFlag::FirstSegment,
                SequenceFlags::LastSegment => ccsds_spacepacket::types::SeqFlag::LastSegment,
                SequenceFlags::Unsegmented => ccsds_spacepacket::types::SeqFlag::Unsegmented
            };
            SpHeader {
                version: version_to_set,
                packet_type,
                sec_header_flag,
                app_proc_id: packet_id.apid,
                sequence_flags,
                sequence_count: psc.ssc,
                data_length: data_len
            }
        }

        #[inline]
        fn version(&self) -> u8 {
            self.version
        }

        #[inline]
        fn packet_id(&self) -> PacketId {
            PacketId {
                ptype: PacketType::try_from(self.packet_type as u8).unwrap(),
                apid: self.app_proc_id,
                sec_header_flag: self.sec_header_flag as u8 != 0
            }
        }

        #[inline]
        fn psc(&self) -> PacketSequenceCtrl {
            PacketSequenceCtrl {
                seq_flags: SequenceFlags::try_from(self.sequence_flags as u8).unwrap(),
                ssc: self.sequence_count
            }
        }

        #[inline]
        fn data_len(&self) -> u16 {
            self.data_length
        }
    }

    sph_from_other!(SpHeader, sp::srd::SpHeader);
    sph_from_other!(SpHeader, sp::zc::SpHeader);
}
*/

/*
#[test]
fn test_deser_to_raw_packed_deku() {
    let sp_header = SpHeader::tc(0x42, 12).expect("Error creating SP header");
    // TODO: Wait with these tests until KubOS merged
    //       https://github.com/KubOS-Preservation-Group/ccsds-spacepacket/pull/14
    let _deku_header =
        deku::SpHeader::try_from(sp_header).expect("Error creating Deku Sp Header");
    // deku_header.to_bytes().unwrap();
}
 */
