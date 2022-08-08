use crate::error::{FsrcErrorRaw, FsrcGroupIds};
use spacepackets::ecss::PusError;
use spacepackets::tc::PusTc;
use spacepackets::{PacketError, SpHeader};

pub mod ccsds_distrib;
pub mod pus_distrib;

const RAW_PACKET_ERROR: &str = "raw-tmtc";
const _CCSDS_ERROR: &str = "ccsds-tmtc";
const _PUS_ERROR: &str = "pus-tmtc";

// TODO: A macro for general and unknown errors would be nice
const FROM_BYTES_SLICE_TOO_SMALL_ERROR: FsrcErrorRaw = FsrcErrorRaw::new(
    FsrcGroupIds::Tmtc as u8,
    0,
    RAW_PACKET_ERROR,
    "FROM_BYTES_SLICE_TOO_SMALL_ERROR",
);

const FROM_BYTES_ZEROCOPY_ERROR: FsrcErrorRaw = FsrcErrorRaw::new(
    FsrcGroupIds::Tmtc as u8,
    1,
    RAW_PACKET_ERROR,
    "FROM_BYTES_ZEROCOPY_ERROR",
);

pub trait ReceivesTc {
    fn pass_tc(&mut self, tc_raw: &[u8]);
}

pub trait ReceivesCcsds {
    fn pass_ccsds(&mut self, header: &SpHeader, tm_raw: &[u8]) -> Result<(), PacketError>;
}

pub trait ReceivesPus {
    fn pass_pus(&mut self, pus_tc: &PusTc) -> Result<(), PusError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SimpleStdErrorHandler;
    use crate::tmtc::ccsds_distrib::{CcsdsDistributor, HandlesPacketForApid};
    use spacepackets::CcsdsPacket;
    use std::collections::HashMap;

    #[derive(Copy, Clone)]
    struct DummyCcsdsHandler {}

    impl ReceivesCcsds for DummyCcsdsHandler {
        fn pass_ccsds(&mut self, header: &SpHeader, tm_raw: &[u8]) -> Result<(), PacketError> {
            println!("CCSDS packet with header {:?} received", header);
            println!("Raw data with len {}: {:x?}", tm_raw.len(), tm_raw);
            Ok(())
        }
    }
    struct ApidHandler {
        handler_map: HashMap<u16, Box<dyn ReceivesCcsds>>,
    }

    impl ApidHandler {
        pub fn add_ccsds_handler(&mut self, apid: u16, ccsds_receiver: Box<dyn ReceivesCcsds>) {
            // TODO: Error handling
            self.handler_map.insert(apid, ccsds_receiver);
        }
    }
    impl HandlesPacketForApid for ApidHandler {
        fn get_apid_handler(&mut self, apid: u16) -> Option<&mut Box<dyn ReceivesCcsds>> {
            self.handler_map.get_mut(&apid)
        }

        fn handle_unknown_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) {
            println!("Packet with unknown APID {} received", sp_header.apid());
            println!("Packet with len {}: {:x?}", tc_raw.len(), tc_raw);
        }
    }
    #[test]
    fn test_distribs() {
        let ccsds_handler = DummyCcsdsHandler {};
        let mut apid_handler = ApidHandler {
            handler_map: HashMap::new(),
        };
        let error_handler = SimpleStdErrorHandler {};
        apid_handler.add_ccsds_handler(0, Box::new(ccsds_handler));
        let mut ccsds_distrib =
            CcsdsDistributor::new(Box::new(apid_handler), Box::new(error_handler));
        let mut sph = SpHeader::tc(0, 0x34, 0).unwrap();
        let pus_tc = PusTc::new_simple(&mut sph, 17, 1, None, true);
        let mut test_buf: [u8; 32] = [0; 32];
        pus_tc
            .write_to(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        ccsds_distrib.pass_tc(&test_buf);
    }
}
