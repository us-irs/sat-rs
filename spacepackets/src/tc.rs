use std::mem::size_of;

type CrcType = u16;
/// PUS C secondary header length is fixed
pub const PUC_TC_SECONDARY_HEADER_LEN: usize = size_of::<zc::PusTcDataFieldHeader>();
pub const PUS_TC_MIN_LEN_WITHOUT_APP_DATA: usize =
    size_of::<crate::zc::SpHeader>() + PUC_TC_SECONDARY_HEADER_LEN + size_of::<CrcType>();

pub mod zc {
    use crate::ecss::{PusError, PusVersion};
    use crate::tc::srd;
    use zerocopy::{AsBytes, FromBytes, NetworkEndian, Unaligned, U16};

    #[derive(FromBytes, AsBytes, Unaligned)]
    #[repr(C)]
    pub struct PusTcDataFieldHeader {
        version_ack: u8,
        service: u8,
        subservice: u8,
        source_id: U16<NetworkEndian>,
    }

    impl TryFrom<srd::PusTcDataFieldHeader> for PusTcDataFieldHeader {
        type Error = PusError;
        fn try_from(value: srd::PusTcDataFieldHeader) -> Result<Self, Self::Error> {
            if value.version != PusVersion::PusC {
                return Err(PusError::VersionNotSupported(value.version));
            }
            Ok(PusTcDataFieldHeader {
                version_ack: ((value.version as u8) << 4) | value.ack,
                service: value.service,
                subservice: value.subservice,
                source_id: U16::from(value.source_id),
            })
        }
    }

    impl PusTcDataFieldHeader {
        pub fn to_bytes(&self, slice: &mut (impl AsMut<[u8]> + ?Sized)) -> Option<()> {
            self.write_to(slice.as_mut())
        }
    }
}

pub mod srd {
    use crate::ecss::{PusPacket, PusVersion, CRC_CCITT_FALSE};
    use crate::srd::SpHeader;
    use crate::{CcsdsPacket, PacketError, PacketId, PacketSequenceCtrl, PacketType};
    use delegate::delegate;
    use serde::{Deserialize, Serialize};
    use std::mem::size_of;
    use zerocopy::AsBytes;

    #[derive(PartialEq, Copy, Clone, Serialize, Deserialize)]
    pub struct PusTcDataFieldHeader {
        pub service: u8,
        pub subservice: u8,
        pub source_id: u16,
        pub ack: u8,
        pub version: PusVersion,
    }

    impl PusTcDataFieldHeader {
        pub fn new(service: u8, subservice: u8, ack: u8) -> Self {
            PusTcDataFieldHeader {
                service,
                subservice,
                ack,
                source_id: 0,
                version: PusVersion::PusC,
            }
        }
    }

    #[derive(PartialEq, Copy, Clone, Serialize, Deserialize)]
    pub struct PusTc<'slice> {
        pub sph: SpHeader,
        pub data_field_header: PusTcDataFieldHeader,
        #[serde(skip)]
        raw_data: Option<&'slice [u8]>,
        app_data: Option<&'slice [u8]>,
        crc16: Option<u16>,
    }

    impl<'slice> PusTc<'slice> {
        pub fn new(
            sph: &mut SpHeader,
            service: u8,
            subservice: u8,
            app_data: Option<&'slice [u8]>,
        ) -> Self {
            sph.packet_id.ptype = PacketType::Tc;
            PusTc {
                sph: *sph,
                raw_data: None,
                app_data,
                data_field_header: PusTcDataFieldHeader::new(service, subservice, 0b1111),
                crc16: None,
            }
        }

        pub fn len_packed(&self) -> usize {
            let mut length = super::PUS_TC_MIN_LEN_WITHOUT_APP_DATA;
            if let Some(app_data) = self.app_data {
                length += app_data.len();
            }
            length
        }

        /// Calculate the CCSDS space packet data length field and sets it
        pub fn set_ccsds_data_len(&mut self) {
            self.sph.data_len =
                self.len_packed() as u16 - size_of::<crate::zc::SpHeader>() as u16 - 1;
        }

        pub fn calc_crc16(&mut self) {
            let mut digest = CRC_CCITT_FALSE.digest();
            let sph_zc = crate::zc::SpHeader::from(self.sph);
            digest.update(sph_zc.as_bytes());
            let pus_tc_header =
                super::zc::PusTcDataFieldHeader::try_from(self.data_field_header).unwrap();
            digest.update(pus_tc_header.as_bytes());
            if let Some(app_data) = self.app_data {
                digest.update(app_data);
            }
            self.crc16 = Some(digest.finalize())
        }

        /// This function updates two important internal fields: The CCSDS packet length in the
        /// space packet header and the CRC16 field. This function should be called before
        /// the TC packet is serialized
        pub fn update_packet_fields(&mut self) {
            self.set_ccsds_data_len();
            self.calc_crc16();
        }

        pub fn copy_to_buf(
            &self,
            slice: &mut (impl AsMut<[u8]> + ?Sized),
        ) -> Result<usize, PacketError> {
            if self.crc16.is_none() {
                return Err(PacketError::CrcCalculationMissing);
            }
            let mut_slice = slice.as_mut();
            let mut curr_idx = 0;
            let sph_zc = crate::zc::SpHeader::from(self.sph);
            let tc_header_len = size_of::<super::zc::PusTcDataFieldHeader>();
            let mut total_size = super::PUS_TC_MIN_LEN_WITHOUT_APP_DATA;
            if let Some(app_data) = self.app_data {
                total_size += app_data.len();
            };
            if total_size > mut_slice.len() {
                return Err(PacketError::ToBytesSliceTooSmall(total_size));
            }
            sph_zc
                .to_bytes(&mut mut_slice[curr_idx..curr_idx + 6])
                .ok_or(PacketError::ToBytesZeroCopyError)?;
            curr_idx += 6;
            // The PUS version is hardcoded to PUS C
            let pus_tc_header =
                super::zc::PusTcDataFieldHeader::try_from(self.data_field_header).unwrap();

            pus_tc_header
                .to_bytes(&mut mut_slice[curr_idx..curr_idx + tc_header_len])
                .ok_or(PacketError::ToBytesZeroCopyError)?;
            curr_idx += tc_header_len;
            if let Some(app_data) = self.app_data {
                mut_slice[curr_idx..curr_idx + app_data.len()].copy_from_slice(app_data);
                curr_idx += app_data.len();
            }
            mut_slice[curr_idx..curr_idx + 2]
                .copy_from_slice(self.crc16.unwrap().to_ne_bytes().as_slice());
            curr_idx += 2;
            Ok(curr_idx)
        }

        pub fn append_to_vec(&self, vec: &mut Vec<u8>) -> Result<usize, PacketError> {
            if self.crc16.is_none() {
                return Err(PacketError::CrcCalculationMissing);
            }
            let sph_zc = crate::zc::SpHeader::from(self.sph);
            let mut appended_len = super::PUS_TC_MIN_LEN_WITHOUT_APP_DATA;
            if let Some(app_data) = self.app_data {
                appended_len += app_data.len();
            };
            vec.extend_from_slice(sph_zc.as_bytes());
            // The PUS version is hardcoded to PUS C
            let pus_tc_header =
                super::zc::PusTcDataFieldHeader::try_from(self.data_field_header).unwrap();
            vec.extend_from_slice(pus_tc_header.as_bytes());
            if let Some(app_data) = self.app_data {
                vec.extend_from_slice(app_data);
            }
            vec.extend_from_slice(self.crc16.unwrap().to_ne_bytes().as_slice());
            Ok(appended_len)
        }
    }

    //noinspection RsTraitImplementation
    impl CcsdsPacket for PusTc<'_> {
        delegate!(to self.sph {
            fn version(&self) -> u8;
            fn packet_id(&self) -> PacketId;
            fn psc(&self) -> PacketSequenceCtrl;
            fn data_len(&self) -> u16;
        });
    }

    impl PusPacket for PusTc<'_> {
        fn service(&self) -> u8 {
            self.data_field_header.service
        }

        fn subservice(&self) -> u8 {
            self.data_field_header.subservice
        }

        fn source_id(&self) -> u16 {
            self.data_field_header.source_id
        }

        fn ack_flags(&self) -> u8 {
            self.data_field_header.ack
        }

        fn user_data(&self) -> Option<&[u8]> {
            self.app_data
        }

        fn crc16(&self) -> Option<u16> {
            self.crc16
        }

        fn verify(&mut self) -> bool {
            let mut digest = CRC_CCITT_FALSE.digest();
            if self.raw_data.is_none() {
                return false;
            }
            digest.update(self.raw_data.unwrap().as_ref());
            if digest.finalize() == 0 {
                return true;
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::srd::SpHeader;
    use crate::tc::srd::PusTc;
    use postcard::to_stdvec;

    #[test]
    fn test_tc() {
        let mut sph = SpHeader::tc(0x42, 12).unwrap();
        let mut pus_tc = PusTc::new(&mut sph, 1, 1, None);
        let _out = to_stdvec(&pus_tc).unwrap();
        let mut test_buf = [0; 32];
        pus_tc.update_packet_fields();
        assert_eq!(pus_tc.len_packed(), 13);
        let size = pus_tc
            .copy_to_buf(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        println!("Test buffer: {:02x?} with {size} written bytes", test_buf);

        let mut test_vec = Vec::new();
        let size = pus_tc
            .append_to_vec(&mut test_vec)
            .expect("Error writing TC to vector");
        println!("Test Vector: {:02x?} with {size} written bytes", test_vec);
    }
}
