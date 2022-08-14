//! ECSS PUS packet routing components.
//!
//! The routing components consist of two core components:
//!  1. [PusDistributor] component which dispatches received packets to a user-provided handler.
//!  2. [PusServiceProvider] trait which should be implemented by the user-provided PUS packet
//!     handler.
//!
//! The [PusDistributor] implements the [ReceivesEcssPusTc], [ReceivesCcsdsTc] and the [ReceivesTc]
//! trait which allows to pass raw packets, CCSDS packets and PUS TC packets into it.
//! Upon receiving a packet, it performs the following steps:
//!
//! 1. It tries to extract the [SpHeader] and [PusTc] objects from the raw bytestream. If this
//!    process fails, a [PusDistribError::PusError] is returned to the user.
//! 2. If it was possible to extract both components, the packet will be passed to the
//!    [PusServiceProvider::handle_pus_tc_packet] method provided by the user.
//!
//! # Example
//!
//! ```rust
//! use fsrc_core::tmtc::pus_distrib::{PusDistributor, PusServiceProvider};
//! use fsrc_core::tmtc::ReceivesTc;
//! use spacepackets::SpHeader;
//! use spacepackets::tc::PusTc;
//! struct ConcretePusHandler {
//!     handler_call_count: u32
//! }
//!
//! impl PusServiceProvider for ConcretePusHandler {
//!     type Error = ();
//!     fn handle_pus_tc_packet(&mut self, service: u8, header: &SpHeader, pus_tc: &PusTc) -> Result<(), Self::Error> {
//!         assert_eq!(service, 17);
//!         assert_eq!(pus_tc.len_packed(), 13);
//!         self.handler_call_count += 1;
//!         Ok(())
//!     }
//! }
//!
//! let service_handler = ConcretePusHandler {
//!     handler_call_count: 0
//! };
//! let mut pus_distributor = PusDistributor::new(Box::new(service_handler));
//!
//! // Create and pass PUS telecommand with a valid APID
//! let mut space_packet_header = SpHeader::tc(0x002, 0x34, 0).unwrap();
//! let mut pus_tc = PusTc::new_simple(&mut space_packet_header, 17, 1, None, true);
//! let mut test_buf: [u8; 32] = [0; 32];
//! let mut size = pus_tc
//!     .write_to(test_buf.as_mut_slice())
//!     .expect("Error writing TC to buffer");
//! let tc_slice = &test_buf[0..size];
//!
//! pus_distributor.pass_tc(tc_slice).expect("Passing PUS telecommand failed");
//!
//! // User helper function to retrieve concrete class
//! let concrete_handler_ref: &ConcretePusHandler = pus_distributor
//!     .service_provider_ref()
//!     .expect("Casting back to concrete type failed");
//! assert_eq!(concrete_handler_ref.handler_call_count, 1);
//! ```
use crate::tmtc::{ReceivesCcsdsTc, ReceivesEcssPusTc, ReceivesTc};
use downcast_rs::Downcast;
use spacepackets::ecss::{PusError, PusPacket};
use spacepackets::tc::PusTc;
use spacepackets::SpHeader;

pub trait PusServiceProvider: Downcast {
    type Error;
    fn handle_pus_tc_packet(
        &mut self,
        service: u8,
        header: &SpHeader,
        pus_tc: &PusTc,
    ) -> Result<(), Self::Error>;
}
downcast_rs::impl_downcast!(PusServiceProvider assoc Error);

pub struct PusDistributor<E> {
    pub service_provider: Box<dyn PusServiceProvider<Error = E>>,
}

impl<E> PusDistributor<E> {
    pub fn new(service_provider: Box<dyn PusServiceProvider<Error = E>>) -> Self {
        PusDistributor {
            service_provider
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum PusDistribError<E> {
    CustomError(E),
    PusError(PusError),
}

impl<E> ReceivesTc for PusDistributor<E> {
    type Error = PusDistribError<E>;
    fn pass_tc(&mut self, tm_raw: &[u8]) -> Result<(), Self::Error> {
        // Convert to ccsds and call pass_ccsds
        let sp_header = SpHeader::from_raw_slice(tm_raw)
            .map_err(|e| PusDistribError::PusError(PusError::PacketError(e)))?;
        self.pass_ccsds(&sp_header, tm_raw)
    }
}

impl<E> ReceivesCcsdsTc for PusDistributor<E> {
    type Error = PusDistribError<E>;
    fn pass_ccsds(&mut self, header: &SpHeader, tm_raw: &[u8]) -> Result<(), Self::Error> {
        let (tc, _) =
            PusTc::new_from_raw_slice(tm_raw).map_err(|e| PusDistribError::PusError(e))?;
        self.pass_pus_tc(header, &tc)
    }
}

impl<E> ReceivesEcssPusTc for PusDistributor<E> {
    type Error = PusDistribError<E>;
    fn pass_pus_tc(&mut self, header: &SpHeader, pus_tc: &PusTc) -> Result<(), Self::Error> {
        self.service_provider
            .handle_pus_tc_packet(pus_tc.service(), header, pus_tc)
            .map_err(|e| PusDistribError::CustomError(e))
    }
}

impl<E: 'static> PusDistributor<E> {
    pub fn service_provider_ref<T: PusServiceProvider<Error = E>>(&self) -> Option<&T> {
        self.service_provider.downcast_ref::<T>()
    }

    pub fn service_provider_mut<T: PusServiceProvider<Error = E>>(&mut self) -> Option<&mut T> {
        self.service_provider.downcast_mut::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SimpleStdErrorHandler;
    use crate::tmtc::ccsds_distrib::tests::{
        generate_ping_tc, BasicApidHandlerOwnedQueue, BasicApidHandlerSharedQueue,
    };
    use crate::tmtc::ccsds_distrib::{ApidPacketHandler, CcsdsDistributor};
    use spacepackets::ecss::PusError;
    use spacepackets::tc::PusTc;
    use spacepackets::CcsdsPacket;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct PusHandlerSharedQueue {
        pub pus_queue: Arc<Mutex<VecDeque<(u8, u16, Vec<u8>)>>>,
    }

    #[derive(Default)]
    struct PusHandlerOwnedQueue {
        pub pus_queue: VecDeque<(u8, u16, Vec<u8>)>,
    }

    impl PusServiceProvider for PusHandlerSharedQueue {
        type Error = PusError;
        fn handle_pus_tc_packet(
            &mut self,
            service: u8,
            sp_header: &SpHeader,
            pus_tc: &PusTc,
        ) -> Result<(), Self::Error> {
            let mut vec: Vec<u8> = Vec::new();
            pus_tc.append_to_vec(&mut vec)?;
            Ok(self
                .pus_queue
                .lock()
                .expect("Mutex lock failed")
                .push_back((service, sp_header.apid(), vec)))
        }
    }

    impl PusServiceProvider for PusHandlerOwnedQueue {
        type Error = PusError;
        fn handle_pus_tc_packet(
            &mut self,
            service: u8,
            sp_header: &SpHeader,
            pus_tc: &PusTc,
        ) -> Result<(), Self::Error> {
            let mut vec: Vec<u8> = Vec::new();
            pus_tc.append_to_vec(&mut vec)?;
            Ok(self.pus_queue.push_back((service, sp_header.apid(), vec)))
        }
    }

    struct ApidHandlerShared {
        pub pus_distrib: PusDistributor<PusError>,
        pub handler_base: BasicApidHandlerSharedQueue,
    }

    struct ApidHandlerOwned {
        pub pus_distrib: PusDistributor<PusError>,
        handler_base: BasicApidHandlerOwnedQueue,
    }

    macro_rules! apid_handler_impl {
        () => {
            type Error = PusError;

            fn valid_apids(&self) -> &'static [u16] {
                &[0x000, 0x002]
            }

            fn handle_known_apid(
                &mut self,
                sp_header: &SpHeader,
                tc_raw: &[u8],
            ) -> Result<(), Self::Error> {
                self.handler_base
                    .handle_known_apid(&sp_header, tc_raw)
                    .ok()
                    .expect("Unexpected error");
                match self.pus_distrib.pass_ccsds(&sp_header, tc_raw) {
                    Ok(_) => Ok(()),
                    Err(e) => match e {
                        PusDistribError::CustomError(_) => Ok(()),
                        PusDistribError::PusError(e) => Err(e),
                    },
                }
            }

            fn handle_unknown_apid(
                &mut self,
                sp_header: &SpHeader,
                tc_raw: &[u8],
            ) -> Result<(), Self::Error> {
                self.handler_base
                    .handle_unknown_apid(&sp_header, tc_raw)
                    .ok()
                    .expect("Unexpected error");
                Ok(())
            }
        };
    }

    impl ApidPacketHandler for ApidHandlerOwned {
        apid_handler_impl!();
    }

    impl ApidPacketHandler for ApidHandlerShared {
        apid_handler_impl!();
    }

    #[test]
    fn test_pus_distribution() {
        let known_packet_queue = Arc::new(Mutex::default());
        let unknown_packet_queue = Arc::new(Mutex::default());
        let pus_queue = Arc::new(Mutex::default());
        let pus_handler = PusHandlerSharedQueue {
            pus_queue: pus_queue.clone(),
        };
        let handler_base = BasicApidHandlerSharedQueue {
            known_packet_queue: known_packet_queue.clone(),
            unknown_packet_queue: unknown_packet_queue.clone(),
        };

        let error_handler = SimpleStdErrorHandler {};
        let pus_distrib = PusDistributor {
            service_provider: Box::new(pus_handler),
        };

        let apid_handler = ApidHandlerShared {
            pus_distrib,
            handler_base,
        };
        let mut ccsds_distrib =
            CcsdsDistributor::new(Box::new(apid_handler), Box::new(error_handler));
        let mut test_buf: [u8; 32] = [0; 32];
        let tc_slice = generate_ping_tc(test_buf.as_mut_slice());

        // Pass packet to distributor
        ccsds_distrib
            .pass_tc(tc_slice)
            .expect("Passing TC slice failed");
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

    #[test]
    fn test_as_any_cast() {
        let pus_handler = PusHandlerOwnedQueue::default();
        let handler_base = BasicApidHandlerOwnedQueue::default();
        let error_handler = SimpleStdErrorHandler {};
        let pus_distrib = PusDistributor {
            service_provider: Box::new(pus_handler),
        };

        let apid_handler = ApidHandlerOwned {
            pus_distrib,
            handler_base,
        };
        let mut ccsds_distrib =
            CcsdsDistributor::new(Box::new(apid_handler), Box::new(error_handler));

        let mut test_buf: [u8; 32] = [0; 32];
        let tc_slice = generate_ping_tc(test_buf.as_mut_slice());

        ccsds_distrib
            .pass_tc(tc_slice)
            .expect("Passing TC slice failed");

        let apid_handler_casted_back: &mut ApidHandlerOwned = ccsds_distrib
            .apid_handler_mut()
            .expect("Cast to concrete type ApidHandler failed");
        assert!(!apid_handler_casted_back
            .handler_base
            .known_packet_queue
            .is_empty());
        let handler_casted_back: &mut PusHandlerOwnedQueue = apid_handler_casted_back
            .pus_distrib
            .service_provider_mut()
            .expect("Cast to concrete type PusHandlerOwnedQueue failed");
        assert!(!handler_casted_back.pus_queue.is_empty());
        let (service, apid, packet_raw) = handler_casted_back.pus_queue.pop_front().unwrap();
        assert_eq!(service, 17);
        assert_eq!(apid, 0x002);
        assert_eq!(packet_raw.as_slice(), tc_slice);
    }
}
