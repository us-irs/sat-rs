//! CCSDS packet routing components.
//!
//! The routing components consist of two core components:
//!  1. [CcsdsDistributor] component which dispatches received packets to a user-provided handler
//!  2. [CcsdsPacketHandler] trait which should be implemented by the user-provided packet handler.
//!
//! The [CcsdsDistributor] implements the [ReceivesCcsdsTc] and [ReceivesTcCore] trait which allows to
//! pass raw or CCSDS packets to it. Upon receiving a packet, it performs the following steps:
//!
//! 1. It tries to identify the target Application Process Identifier (APID) based on the
//!    respective CCSDS space packet header field. If that process fails, a [ByteConversionError] is
//!    returned to the user
//! 2. If a valid APID is found and matches one of the APIDs provided by
//!    [CcsdsPacketHandler::valid_apids], it will pass the packet to the user provided
//!    [CcsdsPacketHandler::handle_known_apid] function. If no valid APID is found, the packet
//!    will be passed to the [CcsdsPacketHandler::handle_unknown_apid] function.
//!
//! # Example
//!
//! ```rust
//! use satrs::ValidatorU16Id;
//! use satrs::tmtc::ccsds_distrib::{CcsdsPacketHandler, CcsdsDistributor};
//! use satrs::tmtc::{ReceivesTc, ReceivesTcCore};
//! use spacepackets::{CcsdsPacket, SpHeader};
//! use spacepackets::ecss::WritablePusPacket;
//! use spacepackets::ecss::tc::PusTcCreator;
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
//! impl ValidatorU16Id for ConcreteApidHandler {
//!     fn validate(&self, apid: u16) -> bool { apid == 0x0002 }
//! }
//!
//! impl CcsdsPacketHandler for ConcreteApidHandler {
//!     type Error = ();
//!     fn handle_packet_with_valid_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
//!         assert_eq!(sp_header.apid(), 0x002);
//!         assert_eq!(tc_raw.len(), 13);
//!         self.known_call_count += 1;
//!         Ok(())
//!     }
//!     fn handle_packet_with_unknown_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
//!         assert_eq!(sp_header.apid(), 0x003);
//!         assert_eq!(tc_raw.len(), 13);
//!         self.unknown_call_count += 1;
//!         Ok(())
//!     }
//! }
//!
//! let apid_handler = ConcreteApidHandler::default();
//! let mut ccsds_distributor = CcsdsDistributor::new(apid_handler);
//!
//! // Create and pass PUS telecommand with a valid APID
//! let sp_header = SpHeader::new_for_unseg_tc(0x002, 0x34, 0);
//! let mut pus_tc = PusTcCreator::new_simple(sp_header, 17, 1, &[], true);
//! let mut test_buf: [u8; 32] = [0; 32];
//! let mut size = pus_tc
//!     .write_to_bytes(test_buf.as_mut_slice())
//!     .expect("Error writing TC to buffer");
//! let tc_slice = &test_buf[0..size];
//! ccsds_distributor.pass_tc(&tc_slice).expect("Passing TC slice failed");
//!
//! // Now pass a packet with an unknown APID to the distributor
//! pus_tc.set_apid(0x003);
//! size = pus_tc
//!     .write_to_bytes(test_buf.as_mut_slice())
//!     .expect("Error writing TC to buffer");
//! let tc_slice = &test_buf[0..size];
//! ccsds_distributor.pass_tc(&tc_slice).expect("Passing TC slice failed");
//!
//! // Retrieve the APID handler.
//! let handler_ref = ccsds_distributor.packet_handler();
//! assert_eq!(handler_ref.known_call_count, 1);
//! assert_eq!(handler_ref.unknown_call_count, 1);
//!
//! // Mutable access to the handler.
//! let mutable_handler_ref = ccsds_distributor.packet_handler_mut();
//! mutable_handler_ref.mutable_foo();
//! ```
use crate::{
    tmtc::{ReceivesCcsdsTc, ReceivesTcCore},
    ValidatorU16Id,
};
use core::fmt::{Display, Formatter};
use spacepackets::{ByteConversionError, CcsdsPacket, SpHeader};
#[cfg(feature = "std")]
use std::error::Error;

/// Generic trait for a handler or dispatcher object handling CCSDS packets.
///
/// Users should implement this trait on their custom CCSDS packet handler and then pass a boxed
/// instance of this handler to the [CcsdsDistributor]. The distributor will use the trait
/// interface to dispatch received packets to the user based on the Application Process Identifier
/// (APID) field of the CCSDS packet. The APID will be checked using the generic [ValidatorU16Id]
/// trait.
pub trait CcsdsPacketHandler: ValidatorU16Id {
    type Error;

    fn handle_packet_with_valid_apid(
        &mut self,
        sp_header: &SpHeader,
        tc_raw: &[u8],
    ) -> Result<(), Self::Error>;

    fn handle_packet_with_unknown_apid(
        &mut self,
        sp_header: &SpHeader,
        tc_raw: &[u8],
    ) -> Result<(), Self::Error>;
}

/// The CCSDS distributor dispatches received CCSDS packets to a user provided packet handler.
pub struct CcsdsDistributor<PacketHandler: CcsdsPacketHandler<Error = E>, E> {
    /// User provided APID handler stored as a generic trait object.
    /// It can be cast back to the original concrete type using [Self::packet_handler] or
    /// the [Self::packet_handler_mut] method.
    packet_handler: PacketHandler,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CcsdsError<E> {
    CustomError(E),
    ByteConversionError(ByteConversionError),
}

impl<E: Display> Display for CcsdsError<E> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CustomError(e) => write!(f, "{e}"),
            Self::ByteConversionError(e) => write!(f, "{e}"),
        }
    }
}

#[cfg(feature = "std")]
impl<E: Error> Error for CcsdsError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CustomError(e) => e.source(),
            Self::ByteConversionError(e) => e.source(),
        }
    }
}

impl<PacketHandler: CcsdsPacketHandler<Error = E>, E: 'static> ReceivesCcsdsTc
    for CcsdsDistributor<PacketHandler, E>
{
    type Error = CcsdsError<E>;

    fn pass_ccsds(&mut self, header: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
        self.dispatch_ccsds(header, tc_raw)
    }
}

impl<PacketHandler: CcsdsPacketHandler<Error = E>, E: 'static> ReceivesTcCore
    for CcsdsDistributor<PacketHandler, E>
{
    type Error = CcsdsError<E>;

    fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
        if tc_raw.len() < 7 {
            return Err(CcsdsError::ByteConversionError(
                ByteConversionError::FromSliceTooSmall {
                    found: tc_raw.len(),
                    expected: 7,
                },
            ));
        }
        let (sp_header, _) =
            SpHeader::from_be_bytes(tc_raw).map_err(|e| CcsdsError::ByteConversionError(e))?;
        self.dispatch_ccsds(&sp_header, tc_raw)
    }
}

impl<PacketHandler: CcsdsPacketHandler<Error = E>, E: 'static> CcsdsDistributor<PacketHandler, E> {
    pub fn new(packet_handler: PacketHandler) -> Self {
        CcsdsDistributor { packet_handler }
    }

    pub fn packet_handler(&self) -> &PacketHandler {
        &self.packet_handler
    }

    pub fn packet_handler_mut(&mut self) -> &mut PacketHandler {
        &mut self.packet_handler
    }

    fn dispatch_ccsds(&mut self, sp_header: &SpHeader, tc_raw: &[u8]) -> Result<(), CcsdsError<E>> {
        let valid_apid = self.packet_handler().validate(sp_header.apid());
        if valid_apid {
            self.packet_handler
                .handle_packet_with_valid_apid(sp_header, tc_raw)
                .map_err(|e| CcsdsError::CustomError(e))?;
            return Ok(());
        }
        self.packet_handler
            .handle_packet_with_unknown_apid(sp_header, tc_raw)
            .map_err(|e| CcsdsError::CustomError(e))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tmtc::ccsds_distrib::{CcsdsDistributor, CcsdsPacketHandler};
    use spacepackets::ecss::tc::PusTcCreator;
    use spacepackets::ecss::WritablePusPacket;
    use spacepackets::CcsdsPacket;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::vec::Vec;

    fn is_send<T: Send>(_: &T) {}

    pub fn generate_ping_tc(buf: &mut [u8]) -> &[u8] {
        let sph = SpHeader::new_for_unseg_tc(0x002, 0x34, 0);
        let pus_tc = PusTcCreator::new_simple(sph, 17, 1, &[], true);
        let size = pus_tc
            .write_to_bytes(buf)
            .expect("Error writing TC to buffer");
        assert_eq!(size, 13);
        &buf[0..size]
    }

    pub fn generate_ping_tc_as_vec() -> Vec<u8> {
        let sph = SpHeader::new_for_unseg_tc(0x002, 0x34, 0);
        PusTcCreator::new_simple(sph, 17, 1, &[], true)
            .to_vec()
            .unwrap()
    }

    type SharedPacketQueue = Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>;
    pub struct BasicApidHandlerSharedQueue {
        pub known_packet_queue: SharedPacketQueue,
        pub unknown_packet_queue: SharedPacketQueue,
    }

    #[derive(Default)]
    pub struct BasicApidHandlerOwnedQueue {
        pub known_packet_queue: VecDeque<(u16, Vec<u8>)>,
        pub unknown_packet_queue: VecDeque<(u16, Vec<u8>)>,
    }

    impl ValidatorU16Id for BasicApidHandlerSharedQueue {
        fn validate(&self, packet_id: u16) -> bool {
            [0x000, 0x002].contains(&packet_id)
        }
    }

    impl CcsdsPacketHandler for BasicApidHandlerSharedQueue {
        type Error = ();

        fn handle_packet_with_valid_apid(
            &mut self,
            sp_header: &SpHeader,
            tc_raw: &[u8],
        ) -> Result<(), Self::Error> {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            self.known_packet_queue
                .lock()
                .unwrap()
                .push_back((sp_header.apid(), vec));
            Ok(())
        }

        fn handle_packet_with_unknown_apid(
            &mut self,
            sp_header: &SpHeader,
            tc_raw: &[u8],
        ) -> Result<(), Self::Error> {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            self.unknown_packet_queue
                .lock()
                .unwrap()
                .push_back((sp_header.apid(), vec));
            Ok(())
        }
    }

    impl ValidatorU16Id for BasicApidHandlerOwnedQueue {
        fn validate(&self, packet_id: u16) -> bool {
            [0x000, 0x002].contains(&packet_id)
        }
    }

    impl CcsdsPacketHandler for BasicApidHandlerOwnedQueue {
        type Error = ();

        fn handle_packet_with_valid_apid(
            &mut self,
            sp_header: &SpHeader,
            tc_raw: &[u8],
        ) -> Result<(), Self::Error> {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            self.known_packet_queue.push_back((sp_header.apid(), vec));
            Ok(())
        }

        fn handle_packet_with_unknown_apid(
            &mut self,
            sp_header: &SpHeader,
            tc_raw: &[u8],
        ) -> Result<(), Self::Error> {
            let mut vec = Vec::new();
            vec.extend_from_slice(tc_raw);
            self.unknown_packet_queue.push_back((sp_header.apid(), vec));
            Ok(())
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
        let mut ccsds_distrib = CcsdsDistributor::new(apid_handler);
        is_send(&ccsds_distrib);
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
    fn test_unknown_apid_handling() {
        let apid_handler = BasicApidHandlerOwnedQueue::default();
        let mut ccsds_distrib = CcsdsDistributor::new(apid_handler);
        let sph = SpHeader::new_for_unseg_tc(0x004, 0x34, 0);
        let pus_tc = PusTcCreator::new_simple(sph, 17, 1, &[], true);
        let mut test_buf: [u8; 32] = [0; 32];
        pus_tc
            .write_to_bytes(test_buf.as_mut_slice())
            .expect("Error writing TC to buffer");
        ccsds_distrib.pass_tc(&test_buf).expect("Passing TC failed");
        assert!(ccsds_distrib.packet_handler().known_packet_queue.is_empty());
        let apid_handler = ccsds_distrib.packet_handler_mut();
        let recvd = apid_handler.unknown_packet_queue.pop_front();
        assert!(recvd.is_some());
        let (apid, packet) = recvd.unwrap();
        assert_eq!(apid, 0x004);
        assert_eq!(packet.as_slice(), test_buf);
    }

    #[test]
    fn test_ccsds_distribution() {
        let mut ccsds_distrib = CcsdsDistributor::new(BasicApidHandlerOwnedQueue::default());
        let sph = SpHeader::new_for_unseg_tc(0x002, 0x34, 0);
        let pus_tc = PusTcCreator::new_simple(sph, 17, 1, &[], true);
        let tc_vec = pus_tc.to_vec().unwrap();
        ccsds_distrib
            .pass_ccsds(&sph, &tc_vec)
            .expect("passing CCSDS TC failed");
        let recvd = ccsds_distrib
            .packet_handler_mut()
            .known_packet_queue
            .pop_front();
        assert!(recvd.is_some());
        let recvd = recvd.unwrap();
        assert_eq!(recvd.0, 0x002);
        assert_eq!(recvd.1, tc_vec);
    }

    #[test]
    fn test_distribution_short_packet_fails() {
        let mut ccsds_distrib = CcsdsDistributor::new(BasicApidHandlerOwnedQueue::default());
        let sph = SpHeader::new_for_unseg_tc(0x002, 0x34, 0);
        let pus_tc = PusTcCreator::new_simple(sph, 17, 1, &[], true);
        let tc_vec = pus_tc.to_vec().unwrap();
        let result = ccsds_distrib.pass_tc(&tc_vec[0..6]);
        assert!(result.is_err());
        let error = result.unwrap_err();
        if let CcsdsError::ByteConversionError(ByteConversionError::FromSliceTooSmall {
            found,
            expected,
        }) = error
        {
            assert_eq!(found, 6);
            assert_eq!(expected, 7);
        } else {
            panic!("Unexpected error variant");
        }
    }
}
