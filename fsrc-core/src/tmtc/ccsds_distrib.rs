use crate::any::AsAny;
use crate::error::FsrcErrorHandler;
use crate::tmtc::{ReceivesTc, FROM_BYTES_SLICE_TOO_SMALL_ERROR, FROM_BYTES_ZEROCOPY_ERROR};
use spacepackets::{CcsdsPacket, PacketError, SpHeader};

pub trait ApidPacketHandler: AsAny {
    fn valid_apids(&self) -> &'static [u16];
    fn handle_known_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]);
    fn handle_unknown_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]);
}

pub struct CcsdsDistributor {
    pub apid_handler: Box<dyn ApidPacketHandler>,
    error_handler: Box<dyn FsrcErrorHandler>,
}

impl CcsdsDistributor {
    pub fn new(
        apid_handler: Box<dyn ApidPacketHandler>,
        error_handler: Box<dyn FsrcErrorHandler>,
    ) -> Self {
        CcsdsDistributor {
            apid_handler,
            error_handler,
        }
    }
}

impl ReceivesTc for CcsdsDistributor {
    fn pass_tc(&mut self, tm_raw: &[u8]) {
        let sp_header = match SpHeader::from_raw_slice(tm_raw) {
            Ok(header) => header,
            Err(e) => {
                match e {
                    PacketError::FromBytesSliceTooSmall(missmatch) => {
                        self.error_handler.error_with_two_params(
                            FROM_BYTES_SLICE_TOO_SMALL_ERROR,
                            missmatch.found as u32,
                            missmatch.expected as u32,
                        );
                    }
                    PacketError::FromBytesZeroCopyError => {
                        self.error_handler.error(FROM_BYTES_ZEROCOPY_ERROR);
                    }
                    _ => {
                        // TODO: Unexpected error
                    }
                }
                return;
            }
        };
        let apid = sp_header.apid();
        let valid_apids = self.apid_handler.valid_apids();
        for &valid_apid in valid_apids {
            if valid_apid == apid {
                return self.apid_handler.handle_known_apid(&sp_header, tm_raw);
            }
        }
        self.apid_handler.handle_unknown_apid(&sp_header, tm_raw);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::error::SimpleStdErrorHandler;
    use crate::tmtc::ccsds_distrib::{ApidPacketHandler, CcsdsDistributor};
    use spacepackets::tc::PusTc;
    use spacepackets::CcsdsPacket;
    use std::any::Any;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    pub struct BasicApidHandlerSharedQueue {
        pub known_packet_queue: Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>,
        pub unknown_packet_queue: Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>,
    }

    #[derive(Default)]
    pub struct BasicApidHandlerOwnedQueue {
        pub known_packet_queue: VecDeque<(u16, Vec<u8>)>,
        pub unknown_packet_queue: VecDeque<(u16, Vec<u8>)>,
    }

    impl AsAny for BasicApidHandlerSharedQueue {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_mut_any(&mut self) -> &mut dyn Any {
            self
        }
    }

    impl ApidPacketHandler for BasicApidHandlerSharedQueue {
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

    impl AsAny for BasicApidHandlerOwnedQueue {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_mut_any(&mut self) -> &mut dyn Any {
            self
        }
    }

    impl ApidPacketHandler for BasicApidHandlerOwnedQueue {
        fn valid_apids(&self) -> &'static [u16] {
            &[0x000, 0x002]
        }

        fn handle_known_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            self.known_packet_queue.push_back((sp_header.apid(), vec));
        }

        fn handle_unknown_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            self.unknown_packet_queue.push_back((sp_header.apid(), vec));
        }
    }

    #[test]
    fn test_distribs_known_apid() {
        let known_packet_queue = Arc::new(Mutex::default());
        let unknown_packet_queue = Arc::new(Mutex::default());
        let apid_handler = BasicApidHandlerSharedQueue {
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
        assert!(unknown_packet_queue.lock().unwrap().is_empty());
        assert!(recvd.is_some());
        let (apid, packet) = recvd.unwrap();
        assert_eq!(apid, 0x002);
        assert_eq!(packet.as_slice(), test_buf);
    }

    #[test]
    fn test_distribs_unknown_apid() {
        let known_packet_queue = Arc::new(Mutex::default());
        let unknown_packet_queue = Arc::new(Mutex::default());
        let apid_handler = BasicApidHandlerSharedQueue {
            known_packet_queue: known_packet_queue.clone(),
            unknown_packet_queue: unknown_packet_queue.clone(),
        };
        let error_handler = SimpleStdErrorHandler {};
        let mut ccsds_distrib =
            CcsdsDistributor::new(Box::new(apid_handler), Box::new(error_handler));
        let mut sph = SpHeader::tc(0x004, 0x34, 0).unwrap();
        let pus_tc = PusTc::new_simple(&mut sph, 17, 1, None, true);
        let mut test_buf: [u8; 32] = [0; 32];
        pus_tc
            .write_to(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        ccsds_distrib.pass_tc(&test_buf);
        let recvd = unknown_packet_queue.lock().unwrap().pop_front();
        assert!(known_packet_queue.lock().unwrap().is_empty());
        assert!(recvd.is_some());
        let (apid, packet) = recvd.unwrap();
        assert_eq!(apid, 0x004);
        assert_eq!(packet.as_slice(), test_buf);
    }
}
