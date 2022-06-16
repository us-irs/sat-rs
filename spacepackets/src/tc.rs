use crc::{Crc, CRC_16_IBM_3740};
use serde::{Deserialize, Serialize};

/// All PUS versions. Only PUS C is supported by this library
#[derive(PartialEq, Copy, Clone, Serialize, Deserialize)]
pub enum PusVersion {
    EsaPus = 0,
    PusA = 1,
    PusC = 2,
}

pub const CRC_CCITT_FALSE: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_3740);

pub mod zc {
    use zerocopy::{AsBytes, FromBytes, NetworkEndian, Unaligned, U16};

    #[derive(FromBytes, AsBytes, Unaligned)]
    #[repr(C)]
    pub struct PusTcDataFieldHeader {
        version_ack: u8,
        service: u8,
        subservice: u8,
        source_id: U16<NetworkEndian>,
    }
}

pub mod srd {
    use crate::ecss::PusPacket;
    use crate::srd::SpHeader;
    use crate::tc::{PusVersion, CRC_CCITT_FALSE};
    use crate::{CcsdsPacket, PacketError, PacketId, PacketSequenceCtrl, PacketType};
    use serde::{Deserialize, Serialize};

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

        pub fn write(&mut self, mut slice: impl AsMut<[u8]>) -> Result<(), PacketError> {
            let sph_zc = crate::zc::SpHeader::from(self.sph);
            if slice.as_mut().len() < 6 {
                return Err(PacketError::ToBytesSliceTooSmall(6));
            }
            sph_zc
                .to_bytes(slice)
                .ok_or(PacketError::ToBytesZeroCopyError)?;
            // TODO: Finish impl
            Ok(())
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
        let pus_tc = PusTc::new(&mut sph, 1, 1, None);
        let out = to_stdvec(&pus_tc).unwrap();
        println!("Vector {:#04x?} with length {}", out, out.len());
    }
}
