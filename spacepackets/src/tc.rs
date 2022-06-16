use std::mem::size_of;

pub const PUS_TC_MIN_LEN_WITHOUT_APP_DATA: usize =
    size_of::<crate::zc::SpHeader>() + size_of::<zc::PusTcDataFieldHeader>();

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

        pub fn copy_to_buf(
            &mut self,
            slice: &mut (impl AsMut<[u8]> + ?Sized),
        ) -> Result<usize, PacketError> {
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
            Ok(curr_idx)
        }

        pub fn append_to_vec(&mut self, vec: &mut Vec<u8>) -> Result<usize, PacketError> {
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
            Ok(appended_len)
        }
    }
    impl CcsdsPacket for PusTc<'_> {
        fn version(&self) -> u8 {
            self.sph.version
        }

        fn packet_id(&self) -> PacketId {
            self.sph.packet_id
        }

        fn psc(&self) -> PacketSequenceCtrl {
            self.sph.psc
        }

        fn data_len(&self) -> u16 {
            self.sph.data_len
        }
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
        let out = to_stdvec(&pus_tc).unwrap();
        println!("Vector {:#04x?} with length {}", out, out.len());

        let mut test_buf = [0; 32];
        let size = pus_tc
            .copy_to_buf(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        println!("Test buffer: {:?} with {size} written bytes", test_buf);

        let mut test_vec = Vec::new();
        let size = pus_tc
            .append_to_vec(&mut test_vec)
            .expect("Error writing TC to vector");
        println!("Test Vector: {:?} with {size} written bytes", test_vec);
    }
}
