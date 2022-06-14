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

pub mod srd {
    use crate::sp::ecss::PusPacket;
    use crate::sp::srd::SpHeader;
    use crate::sp::tc::{PusVersion, CRC_CCITT_FALSE};
    use crate::sp::{CcsdsPacket, PacketId, PacketSequenceCtrl};
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

    pub struct PusTc<'slice> {
        pub sph: SpHeader,
        pub data_field_header: PusTcDataFieldHeader,
        raw_data: Option<&'slice [u8]>,
        app_data: Option<&'slice [u8]>,
        crc16: u16,
    }

    impl<'slice> PusTc<'slice> {
        pub fn new(
            sph: &SpHeader,
            service: u8,
            subservice: u8,
            app_data: Option<&'slice [u8]>,
        ) -> Self {
            PusTc {
                sph: sph.clone(),
                raw_data: None,
                app_data,
                data_field_header: PusTcDataFieldHeader::new(service, subservice, 0b1111),
                crc16: 0,
            }
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

        fn crc16(&self) -> u16 {
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
