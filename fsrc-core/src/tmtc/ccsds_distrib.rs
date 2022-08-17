//! CCSDS packet routing components.
//!
//! The routing components consist of two core components:
//!  1. [CcsdsDistributor] component which dispatches received packets to a user-provided handler
//!  2. [ApidPacketHandler] trait which should be implemented by the user-provided packet handler.
//!
//! The [CcsdsDistributor] implements the [ReceivesCcsdsTc] and [ReceivesTc] trait which allows to
//! pass raw or CCSDS packets to it. Upon receiving a packet, it performs the following steps:
//!
//! 1. It tries to identify the target Application Process Identifier (APID) based on the
//!    respective CCSDS space packet header field. If that process fails, a [PacketError] is
//!    returned to the user
//! 2. If a valid APID is found and matches one of the APIDs provided by
//!    [ApidPacketHandler::valid_apids], it will pass the packet to the user provided
//!    [ApidPacketHandler::handle_known_apid] function. If no valid APID is found, the packet
//!    will be passed to the [ApidPacketHandler::handle_unknown_apid] function.
//!
//! # Example
//!
//! ```rust
//! use fsrc_core::tmtc::ccsds_distrib::{ApidPacketHandler, CcsdsDistributor};
//! use fsrc_core::tmtc::ReceivesTc;
//! use spacepackets::{CcsdsPacket, SpHeader};
//! use spacepackets::tc::PusTc;
//!
//! #[derive (Default)]
//! struct ConcreteApidHandler {
//!     known_call_count: u32,
//!     unknown_call_count: u32
//! }
//!
//! impl ConcreteApidHandler {
//!     fn mutable_foo(&mut self) {}
//! }
//!
//! impl ApidPacketHandler for ConcreteApidHandler {
//!     type Error = ();
//!     fn valid_apids(&self) -> &'static [u16] { &[0x002] }
//!     fn handle_known_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
//!         assert_eq!(sp_header.apid(), 0x002);
//!         assert_eq!(tc_raw.len(), 13);
//!         self.known_call_count += 1;
//!         Ok(())
//!     }
//!     fn handle_unknown_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
//!         assert_eq!(sp_header.apid(), 0x003);
//!         assert_eq!(tc_raw.len(), 13);
//!         self.unknown_call_count += 1;
//!         Ok(())
//!     }
//! }
//!
//! let apid_handler = ConcreteApidHandler::default();
//! let mut ccsds_distributor = CcsdsDistributor::new(Box::new(apid_handler));
//!
//! // Create and pass PUS telecommand with a valid APID
//! let mut space_packet_header = SpHeader::tc(0x002, 0x34, 0).unwrap();
//! let mut pus_tc = PusTc::new_simple(&mut space_packet_header, 17, 1, None, true);
//! let mut test_buf: [u8; 32] = [0; 32];
//! let mut size = pus_tc
//!     .write_to(test_buf.as_mut_slice())
//!     .expect("Error writing TC to buffer");
//! let tc_slice = &test_buf[0..size];
//! ccsds_distributor.pass_tc(&tc_slice).expect("Passing TC slice failed");
//!
//! // Now pass a packet with an unknown APID to the distributor
//! pus_tc.set_apid(0x003);
//! size = pus_tc
//!     .write_to(test_buf.as_mut_slice())
//!     .expect("Error writing TC to buffer");
//! let tc_slice = &test_buf[0..size];
//! ccsds_distributor.pass_tc(&tc_slice).expect("Passing TC slice failed");
//!
//! // User helper function to retrieve concrete class
//! let concrete_handler_ref: &ConcreteApidHandler = ccsds_distributor
//!     .apid_handler_ref()
//!     .expect("Casting back to concrete type failed");
//! assert_eq!(concrete_handler_ref.known_call_count, 1);
//! assert_eq!(concrete_handler_ref.unknown_call_count, 1);
//!
//! // It's also possible to retrieve a mutable reference
//! let mutable_ref: &mut ConcreteApidHandler = ccsds_distributor
//!     .apid_handler_mut()
//!     .expect("Casting back to concrete type failed");
//! mutable_ref.mutable_foo();
//! ```
use crate::tmtc::{ReceivesCcsdsTc, ReceivesTc};
use downcast_rs::Downcast;
use spacepackets::{CcsdsPacket, PacketError, SpHeader};

/// Generic trait for a handler or dispatcher object handling CCSDS packets.
///
/// Users should implement this trait on their custom CCSDS packet handler and then pass a boxed
/// instance of this handler to the [CcsdsDistributor]. The distributor will use the trait
/// interface to dispatch received packets to the user based on the Application Process Identifier
/// (APID) field of the CCSDS packet.
///
/// This trait automatically implements the [downcast_rs::Downcast] to allow a more convenient API
/// to cast trait objects back to their concrete type after the handler was passed to the
/// distributor.
pub trait ApidPacketHandler: Downcast {
    type Error;

    fn valid_apids(&self) -> &'static [u16];
    fn handle_known_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8])
        -> Result<(), Self::Error>;
    fn handle_unknown_apid(
        &mut self,
        sp_header: &SpHeader,
        tc_raw: &[u8],
    ) -> Result<(), Self::Error>;
}

downcast_rs::impl_downcast!(ApidPacketHandler assoc Error);

/// The CCSDS distributor dispatches received CCSDS packets to a user provided packet handler.
pub struct CcsdsDistributor<E> {
    /// User provided APID handler stored as a generic trait object.
    /// It can be cast back to the original concrete type using the [Self::apid_handler_ref] or
    /// the [Self::apid_handler_mut] method.
    pub apid_handler: Box<dyn ApidPacketHandler<Error = E>>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CcsdsError<E> {
    CustomError(E),
    PacketError(PacketError),
}

impl<E: 'static> ReceivesCcsdsTc for CcsdsDistributor<E> {
    type Error = CcsdsError<E>;

    fn pass_ccsds(&mut self, header: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
        self.dispatch_ccsds(header, tc_raw)
    }
}

impl<E: 'static> ReceivesTc for CcsdsDistributor<E> {
    type Error = CcsdsError<E>;

    fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
        let sp_header = SpHeader::from_raw_slice(tc_raw).map_err(|e| CcsdsError::PacketError(e))?;
        self.dispatch_ccsds(&sp_header, tc_raw)
    }
}

impl<E: 'static> CcsdsDistributor<E> {
    pub fn new(apid_handler: Box<dyn ApidPacketHandler<Error = E>>) -> Self {
        CcsdsDistributor { apid_handler }
    }

    /// This function can be used to retrieve a reference to the concrete instance of the APID
    /// handler after it was passed to the distributor. See the
    /// [module documentation][crate::tmtc::ccsds_distrib] for an fsrc-example.
    pub fn apid_handler_ref<T: ApidPacketHandler<Error = E>>(&self) -> Option<&T> {
        self.apid_handler.downcast_ref::<T>()
    }

    /// This function can be used to retrieve a mutable reference to the concrete instance of the
    /// APID handler after it was passed to the distributor.
    pub fn apid_handler_mut<T: ApidPacketHandler<Error = E>>(&mut self) -> Option<&mut T> {
        self.apid_handler.downcast_mut::<T>()
    }

    fn dispatch_ccsds(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) -> Result<(), CcsdsError<E>> {
        let apid = sp_header.apid();
        let valid_apids = self.apid_handler.valid_apids();
        for &valid_apid in valid_apids {
            if valid_apid == apid {
                return self
                    .apid_handler
                    .handle_known_apid(sp_header, tc_raw)
                    .map_err(|e| CcsdsError::CustomError(e));
            }
        }
        self.apid_handler
            .handle_unknown_apid(sp_header, tc_raw)
            .map_err(|e| CcsdsError::CustomError(e))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tmtc::ccsds_distrib::{ApidPacketHandler, CcsdsDistributor};
    use spacepackets::tc::PusTc;
    use spacepackets::CcsdsPacket;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    pub fn generate_ping_tc(buf: &mut [u8]) -> &[u8] {
        let mut sph = SpHeader::tc(0x002, 0x34, 0).unwrap();
        let pus_tc = PusTc::new_simple(&mut sph, 17, 1, None, true);
        let size = pus_tc.write_to(buf).expect("Error writing TC to buffer");
        assert_eq!(size, 13);
        &buf[0..size]
    }

    pub struct BasicApidHandlerSharedQueue {
        pub known_packet_queue: Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>,
        pub unknown_packet_queue: Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>,
    }

    #[derive(Default)]
    pub struct BasicApidHandlerOwnedQueue {
        pub known_packet_queue: VecDeque<(u16, Vec<u8>)>,
        pub unknown_packet_queue: VecDeque<(u16, Vec<u8>)>,
    }

    impl ApidPacketHandler for BasicApidHandlerSharedQueue {
        type Error = ();
        fn valid_apids(&self) -> &'static [u16] {
            &[0x000, 0x002]
        }

        fn handle_known_apid(
            &mut self,
            sp_header: &SpHeader,
            tc_raw: &[u8],
        ) -> Result<(), Self::Error> {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            Ok(self
                .known_packet_queue
                .lock()
                .unwrap()
                .push_back((sp_header.apid(), vec)))
        }

        fn handle_unknown_apid(
            &mut self,
            sp_header: &SpHeader,
            tc_raw: &[u8],
        ) -> Result<(), Self::Error> {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            Ok(self
                .unknown_packet_queue
                .lock()
                .unwrap()
                .push_back((sp_header.apid(), vec)))
        }
    }

    impl ApidPacketHandler for BasicApidHandlerOwnedQueue {
        type Error = ();

        fn valid_apids(&self) -> &'static [u16] {
            &[0x000, 0x002]
        }

        fn handle_known_apid(
            &mut self,
            sp_header: &SpHeader,
            tc_raw: &[u8],
        ) -> Result<(), Self::Error> {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            Ok(self.known_packet_queue.push_back((sp_header.apid(), vec)))
        }

        fn handle_unknown_apid(
            &mut self,
            sp_header: &SpHeader,
            tc_raw: &[u8],
        ) -> Result<(), Self::Error> {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            Ok(self.unknown_packet_queue.push_back((sp_header.apid(), vec)))
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
        let mut ccsds_distrib = CcsdsDistributor::new(Box::new(apid_handler));
        let mut test_buf: [u8; 32] = [0; 32];
        let tc_slice = generate_ping_tc(test_buf.as_mut_slice());

        ccsds_distrib.pass_tc(tc_slice).expect("Passing TC failed");
        let recvd = known_packet_queue.lock().unwrap().pop_front();
        assert!(unknown_packet_queue.lock().unwrap().is_empty());
        assert!(recvd.is_some());
        let (apid, packet) = recvd.unwrap();
        assert_eq!(apid, 0x002);
        assert_eq!(packet, tc_slice);
    }

    #[test]
    fn test_distribs_unknown_apid() {
        let known_packet_queue = Arc::new(Mutex::default());
        let unknown_packet_queue = Arc::new(Mutex::default());
        let apid_handler = BasicApidHandlerSharedQueue {
            known_packet_queue: known_packet_queue.clone(),
            unknown_packet_queue: unknown_packet_queue.clone(),
        };
        let mut ccsds_distrib = CcsdsDistributor::new(Box::new(apid_handler));
        let mut sph = SpHeader::tc(0x004, 0x34, 0).unwrap();
        let pus_tc = PusTc::new_simple(&mut sph, 17, 1, None, true);
        let mut test_buf: [u8; 32] = [0; 32];
        pus_tc
            .write_to(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        ccsds_distrib.pass_tc(&test_buf).expect("Passing TC failed");
        let recvd = unknown_packet_queue.lock().unwrap().pop_front();
        assert!(known_packet_queue.lock().unwrap().is_empty());
        assert!(recvd.is_some());
        let (apid, packet) = recvd.unwrap();
        assert_eq!(apid, 0x004);
        assert_eq!(packet.as_slice(), test_buf);
    }
}
