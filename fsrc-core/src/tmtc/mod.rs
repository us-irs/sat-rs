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

pub trait ReceivesCcsdsTc {
    fn pass_ccsds(&mut self, header: &SpHeader, tc_raw: &[u8]) -> Result<(), PacketError>;
}

pub trait ReceivesPusTc {
    fn pass_pus(&mut self, pus_tc: &PusTc) -> Result<(), PusError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SimpleStdErrorHandler;
    use crate::tmtc::ccsds_distrib::{CcsdsDistributor, HandlesPacketForApid};
    use spacepackets::CcsdsPacket;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct ApidHandler {
        known_packet_queue: Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>,
        unknown_packet_queue: Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>,
    }

    impl HandlesPacketForApid for ApidHandler {
        fn valid_apids(&self) -> &'static [u16] {
            &[0x000, 0x002]
        }

        fn handle_known_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            self.known_packet_queue
                .lock()
                .unwrap()
                .push_back((sp_header.apid(), vec));
        }

        fn handle_unknown_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            self.unknown_packet_queue
                .lock()
                .unwrap()
                .push_back((sp_header.apid(), vec));
        }
    }

    #[test]
    fn test_distribs_known_apid() {
        let known_packet_queue = Arc::new(Mutex::default());
        let unknown_packet_queue = Arc::new(Mutex::default());
        let apid_handler = ApidHandler {
            known_packet_queue: known_packet_queue.clone(),
            unknown_packet_queue: unknown_packet_queue.clone(),
        };
        let error_handler = SimpleStdErrorHandler {};
        let mut ccsds_distrib =
            CcsdsDistributor::new(Box::new(apid_handler), Box::new(error_handler));
        let mut sph = SpHeader::tc(0x002, 0x34, 0).unwrap();
        let pus_tc = PusTc::new_simple(&mut sph, 17, 1, None, true);
        let mut test_buf: [u8; 32] = [0; 32];
        pus_tc
            .write_to(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        ccsds_distrib.pass_tc(&test_buf);
        let recvd = known_packet_queue.lock().unwrap().pop_front();
        assert!(recvd.is_some());
        let (apid, packet) = recvd.unwrap();
        assert_eq!(apid, 0x002);
        assert_eq!(packet.as_slice(), test_buf);
    }
}
