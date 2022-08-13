use crate::error::FsrcErrorHandler;
use crate::tmtc::{ReceivesCcsdsTc, ReceivesTc};
use spacepackets::ecss::{PusError, PusPacket};
use spacepackets::tc::PusTc;
use spacepackets::{CcsdsPacket, PacketError, SpHeader};

pub trait PusServiceProvider {
    fn handle_pus_tc_packet(&mut self, service: u8, apid: u16, pus_tc: &PusTc);
}

pub struct PusDistributor {
    service_provider: Box<dyn PusServiceProvider>,
    error_handler: Box<dyn FsrcErrorHandler>,
}

impl ReceivesTc for PusDistributor {
    fn pass_tc(&mut self, tm_raw: &[u8]) {
        // Convert to ccsds and call pass_ccsds
        let sp_header = SpHeader::from_raw_slice(tm_raw).unwrap();
        self.pass_ccsds(&sp_header, tm_raw).unwrap();
    }
}

impl ReceivesCcsdsTc for PusDistributor {
    fn pass_ccsds(&mut self, _header: &SpHeader, tm_raw: &[u8]) -> Result<(), PacketError> {
        // TODO: Better error handling
        let (tc, _) = match PusTc::new_from_raw_slice(tm_raw) {
            Ok(tuple) => tuple,
            Err(e) => {
                match e {
                    PusError::VersionNotSupported(_) => {}
                    PusError::IncorrectCrc(_) => {}
                    PusError::RawDataTooShort(_) => {}
                    PusError::NoRawData => {}
                    PusError::CrcCalculationMissing => {}
                    PusError::PacketError(_) => {}
                }
                return Ok(());
            }
        };

        self.service_provider
            .handle_pus_tc_packet(tc.service(), tc.apid(), &tc);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SimpleStdErrorHandler;
    use crate::tmtc::ccsds_distrib::tests::BasicApidHandler;
    use crate::tmtc::ccsds_distrib::{ApidPacketHandler, CcsdsDistributor};
    use spacepackets::tc::PusTc;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct PusHandler {
        pus_queue: Arc<Mutex<VecDeque<(u8, u16, Vec<u8>)>>>,
    }

    impl PusServiceProvider for PusHandler {
        fn handle_pus_tc_packet(&mut self, service: u8, apid: u16, pus_tc: &PusTc) {
            let mut vec: Vec<u8> = Vec::new();
            pus_tc
                .append_to_vec(&mut vec)
                .expect("Appending raw PUS TC to vector failed");
            self.pus_queue
                .lock()
                .unwrap()
                .push_back((service, apid, vec));
        }
    }

    struct ApidHandler {
        pus_distrib: PusDistributor,
        handler_base: BasicApidHandler,
    }

    impl ApidPacketHandler for ApidHandler {
        fn valid_apids(&self) -> &'static [u16] {
            &[0x000, 0x002]
        }

        fn handle_known_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) {
            self.handler_base.handle_known_apid(&sp_header, tc_raw);
            self.pus_distrib
                .pass_ccsds(&sp_header, tc_raw)
                .expect("Passing PUS packet failed");
        }

        fn handle_unknown_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) {
            self.handler_base.handle_unknown_apid(&sp_header, tc_raw);
        }
    }

    #[test]
    fn test_pus_distribution() {
        let known_packet_queue = Arc::new(Mutex::default());
        let unknown_packet_queue = Arc::new(Mutex::default());
        let pus_queue = Arc::new(Mutex::default());
        let pus_handler = PusHandler {
            pus_queue: pus_queue.clone(),
        };
        let handler_base = BasicApidHandler {
            known_packet_queue: known_packet_queue.clone(),
            unknown_packet_queue: unknown_packet_queue.clone(),
        };

        let error_handler = SimpleStdErrorHandler {};
        let pus_distrib = PusDistributor {
            service_provider: Box::new(pus_handler),
            error_handler: Box::new(error_handler),
        };

        let apid_handler = ApidHandler {
            pus_distrib,
            handler_base,
        };

        let mut ccsds_distrib =
            CcsdsDistributor::new(Box::new(apid_handler), Box::new(error_handler));
        let mut sph = SpHeader::tc(0x002, 0x34, 0).unwrap();
        let pus_tc = PusTc::new_simple(&mut sph, 17, 1, None, true);
        let mut test_buf: [u8; 32] = [0; 32];
        let size = pus_tc
            .write_to(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        let tc_slice = &test_buf[0..size];
        ccsds_distrib.pass_tc(tc_slice);
        let recvd_ccsds = known_packet_queue.lock().unwrap().pop_front();
        assert!(unknown_packet_queue.lock().unwrap().is_empty());
        assert!(recvd_ccsds.is_some());
        let (apid, packet) = recvd_ccsds.unwrap();
        assert_eq!(apid, 0x002);
        assert_eq!(packet.as_slice(), tc_slice);
        let recvd_pus = pus_queue.lock().unwrap().pop_front();
        assert!(recvd_pus.is_some());
        let (service, apid, tc_raw) = recvd_pus.unwrap();
        assert_eq!(service, 17);
        assert_eq!(apid, 0x002);
        assert_eq!(tc_raw, tc_slice);
    }
}
