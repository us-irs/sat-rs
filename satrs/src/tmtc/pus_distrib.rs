//! ECSS PUS packet routing components.
//!
//! The routing components consist of two core components:
//!  1. [PusDistributor] component which dispatches received packets to a user-provided handler.
//!  2. [PusServiceDistributor] trait which should be implemented by the user-provided PUS packet
//!     handler.
//!
//! The [PusDistributor] implements the [ReceivesEcssPusTc], [ReceivesCcsdsTc] and the
//! [ReceivesTcCore] trait which allows to pass raw packets, CCSDS packets and PUS TC packets into
//! it. Upon receiving a packet, it performs the following steps:
//!
//! 1. It tries to extract the [SpHeader] and [spacepackets::ecss::tc::PusTcReader] objects from
//!    the raw bytestream. If this process fails, a [PusDistribError::PusError] is returned to the
//!    user.
//! 2. If it was possible to extract both components, the packet will be passed to the
//!    [PusServiceDistributor::distribute_packet] method provided by the user.
//!
//! # Example
//!
//! ```rust
//! use spacepackets::ecss::WritablePusPacket;
//! use satrs::tmtc::pus_distrib::{PusDistributor, PusServiceDistributor};
//! use satrs::tmtc::{ReceivesTc, ReceivesTcCore};
//! use spacepackets::SpHeader;
//! use spacepackets::ecss::tc::{PusTcCreator, PusTcReader};
//!
//! struct ConcretePusHandler {
//!     handler_call_count: u32
//! }
//!
//! // This is a very simple possible service provider. It increments an internal call count field,
//! // which is used to verify the handler was called
//! impl PusServiceDistributor for ConcretePusHandler {
//!     type Error = ();
//!     fn distribute_packet(&mut self, service: u8, header: &SpHeader, pus_tc: &PusTcReader) -> Result<(), Self::Error> {
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
//! let mut pus_distributor = PusDistributor::new(service_handler);
//!
//! // Create and pass PUS ping telecommand with a valid APID
//! let sp_header = SpHeader::new_for_unseg_tc(0x002, 0x34, 0);
//! let mut pus_tc = PusTcCreator::new_simple(sp_header, 17, 1, &[], true);
//! let mut test_buf: [u8; 32] = [0; 32];
//! let mut size = pus_tc
//!     .write_to_bytes(test_buf.as_mut_slice())
//!     .expect("Error writing TC to buffer");
//! let tc_slice = &test_buf[0..size];
//!
//! pus_distributor.pass_tc(tc_slice).expect("Passing PUS telecommand failed");
//!
//! // User helper function to retrieve concrete class. We check the call count here to verify
//! // that the PUS ping telecommand was routed successfully.
//! let concrete_handler = pus_distributor.service_distributor();
//! assert_eq!(concrete_handler.handler_call_count, 1);
//! ```
use crate::pus::ReceivesEcssPusTc;
use crate::tmtc::{ReceivesCcsdsTc, ReceivesTcCore};
use core::fmt::{Display, Formatter};
use spacepackets::ecss::tc::PusTcReader;
use spacepackets::ecss::{PusError, PusPacket};
use spacepackets::SpHeader;
#[cfg(feature = "std")]
use std::error::Error;

/// Trait for a generic distributor object which can distribute PUS packets based on packet
/// properties like the PUS service, space packet header or any other content of the PUS packet.
pub trait PusServiceDistributor {
    type Error;
    fn distribute_packet(
        &mut self,
        service: u8,
        header: &SpHeader,
        pus_tc: &PusTcReader,
    ) -> Result<(), Self::Error>;
}

/// Generic distributor object which dispatches received packets to a user provided handler.
pub struct PusDistributor<ServiceDistributor: PusServiceDistributor<Error = E>, E> {
    service_distributor: ServiceDistributor,
}

impl<ServiceDistributor: PusServiceDistributor<Error = E>, E>
    PusDistributor<ServiceDistributor, E>
{
    pub fn new(service_provider: ServiceDistributor) -> Self {
        PusDistributor {
            service_distributor: service_provider,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PusDistribError<E> {
    CustomError(E),
    PusError(PusError),
}

impl<E: Display> Display for PusDistribError<E> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            PusDistribError::CustomError(e) => write!(f, "pus distribution error: {e}"),
            PusDistribError::PusError(e) => write!(f, "pus distribution error: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl<E: Error> Error for PusDistribError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CustomError(e) => e.source(),
            Self::PusError(e) => e.source(),
        }
    }
}

impl<ServiceDistributor: PusServiceDistributor<Error = E>, E: 'static> ReceivesTcCore
    for PusDistributor<ServiceDistributor, E>
{
    type Error = PusDistribError<E>;
    fn pass_tc(&mut self, tm_raw: &[u8]) -> Result<(), Self::Error> {
        // Convert to ccsds and call pass_ccsds
        let (sp_header, _) = SpHeader::from_be_bytes(tm_raw)
            .map_err(|e| PusDistribError::PusError(PusError::ByteConversion(e)))?;
        self.pass_ccsds(&sp_header, tm_raw)
    }
}

impl<ServiceDistributor: PusServiceDistributor<Error = E>, E: 'static> ReceivesCcsdsTc
    for PusDistributor<ServiceDistributor, E>
{
    type Error = PusDistribError<E>;
    fn pass_ccsds(&mut self, header: &SpHeader, tm_raw: &[u8]) -> Result<(), Self::Error> {
        let (tc, _) = PusTcReader::new(tm_raw).map_err(|e| PusDistribError::PusError(e))?;
        self.pass_pus_tc(header, &tc)
    }
}

impl<ServiceDistributor: PusServiceDistributor<Error = E>, E: 'static> ReceivesEcssPusTc
    for PusDistributor<ServiceDistributor, E>
{
    type Error = PusDistribError<E>;
    fn pass_pus_tc(&mut self, header: &SpHeader, pus_tc: &PusTcReader) -> Result<(), Self::Error> {
        self.service_distributor
            .distribute_packet(pus_tc.service(), header, pus_tc)
            .map_err(|e| PusDistribError::CustomError(e))
    }
}

impl<ServiceDistributor: PusServiceDistributor<Error = E>, E: 'static>
    PusDistributor<ServiceDistributor, E>
{
    pub fn service_distributor(&self) -> &ServiceDistributor {
        &self.service_distributor
    }

    pub fn service_distributor_mut(&mut self) -> &mut ServiceDistributor {
        &mut self.service_distributor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::GenericSendError;
    use crate::tmtc::ccsds_distrib::tests::{
        generate_ping_tc, generate_ping_tc_as_vec, BasicApidHandlerOwnedQueue,
        BasicApidHandlerSharedQueue,
    };
    use crate::tmtc::ccsds_distrib::{CcsdsDistributor, CcsdsPacketHandler};
    use crate::ValidatorU16Id;
    use alloc::format;
    use alloc::vec::Vec;
    use spacepackets::ecss::PusError;
    use spacepackets::CcsdsPacket;
    #[cfg(feature = "std")]
    use std::collections::VecDeque;
    #[cfg(feature = "std")]
    use std::sync::{Arc, Mutex};

    fn is_send<T: Send>(_: &T) {}

    pub struct PacketInfo {
        pub service: u8,
        pub apid: u16,
        pub packet: Vec<u8>,
    }

    struct PusHandlerSharedQueue(Arc<Mutex<VecDeque<PacketInfo>>>);

    #[derive(Default)]
    struct PusHandlerOwnedQueue(VecDeque<PacketInfo>);

    impl PusServiceDistributor for PusHandlerSharedQueue {
        type Error = PusError;
        fn distribute_packet(
            &mut self,
            service: u8,
            sp_header: &SpHeader,
            pus_tc: &PusTcReader,
        ) -> Result<(), Self::Error> {
            let mut packet: Vec<u8> = Vec::new();
            packet.extend_from_slice(pus_tc.raw_data());
            self.0
                .lock()
                .expect("Mutex lock failed")
                .push_back(PacketInfo {
                    service,
                    apid: sp_header.apid(),
                    packet,
                });
            Ok(())
        }
    }

    impl PusServiceDistributor for PusHandlerOwnedQueue {
        type Error = PusError;
        fn distribute_packet(
            &mut self,
            service: u8,
            sp_header: &SpHeader,
            pus_tc: &PusTcReader,
        ) -> Result<(), Self::Error> {
            let mut packet: Vec<u8> = Vec::new();
            packet.extend_from_slice(pus_tc.raw_data());
            self.0.push_back(PacketInfo {
                service,
                apid: sp_header.apid(),
                packet,
            });
            Ok(())
        }
    }

    struct ApidHandlerShared {
        pub pus_distrib: PusDistributor<PusHandlerSharedQueue, PusError>,
        pub handler_base: BasicApidHandlerSharedQueue,
    }

    struct ApidHandlerOwned {
        pub pus_distrib: PusDistributor<PusHandlerOwnedQueue, PusError>,
        handler_base: BasicApidHandlerOwnedQueue,
    }

    macro_rules! apid_handler_impl {
        () => {
            type Error = PusError;

            fn handle_packet_with_valid_apid(
                &mut self,
                sp_header: &SpHeader,
                tc_raw: &[u8],
            ) -> Result<(), Self::Error> {
                self.handler_base
                    .handle_packet_with_valid_apid(&sp_header, tc_raw)
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

            fn handle_packet_with_unknown_apid(
                &mut self,
                sp_header: &SpHeader,
                tc_raw: &[u8],
            ) -> Result<(), Self::Error> {
                self.handler_base
                    .handle_packet_with_unknown_apid(&sp_header, tc_raw)
                    .ok()
                    .expect("Unexpected error");
                Ok(())
            }
        };
    }

    impl ValidatorU16Id for ApidHandlerOwned {
        fn validate(&self, packet_id: u16) -> bool {
            [0x000, 0x002].contains(&packet_id)
        }
    }

    impl ValidatorU16Id for ApidHandlerShared {
        fn validate(&self, packet_id: u16) -> bool {
            [0x000, 0x002].contains(&packet_id)
        }
    }

    impl CcsdsPacketHandler for ApidHandlerOwned {
        apid_handler_impl!();
    }

    impl CcsdsPacketHandler for ApidHandlerShared {
        apid_handler_impl!();
    }

    #[test]
    fn test_pus_distribution_as_raw_packet() {
        let mut pus_distrib = PusDistributor::new(PusHandlerOwnedQueue::default());
        let tc = generate_ping_tc_as_vec();
        let result = pus_distrib.pass_tc(&tc);
        assert!(result.is_ok());
        assert_eq!(pus_distrib.service_distributor_mut().0.len(), 1);
        let packet_info = pus_distrib.service_distributor_mut().0.pop_front().unwrap();
        assert_eq!(packet_info.service, 17);
        assert_eq!(packet_info.apid, 0x002);
        assert_eq!(packet_info.packet, tc);
    }

    #[test]
    fn test_pus_distribution_combined_handler() {
        let known_packet_queue = Arc::new(Mutex::default());
        let unknown_packet_queue = Arc::new(Mutex::default());
        let pus_queue = Arc::new(Mutex::default());
        let pus_handler = PusHandlerSharedQueue(pus_queue.clone());
        let handler_base = BasicApidHandlerSharedQueue {
            known_packet_queue: known_packet_queue.clone(),
            unknown_packet_queue: unknown_packet_queue.clone(),
        };

        let pus_distrib = PusDistributor::new(pus_handler);
        is_send(&pus_distrib);
        let apid_handler = ApidHandlerShared {
            pus_distrib,
            handler_base,
        };
        let mut ccsds_distrib = CcsdsDistributor::new(apid_handler);
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
        let packet_info = recvd_pus.unwrap();
        assert_eq!(packet_info.service, 17);
        assert_eq!(packet_info.apid, 0x002);
        assert_eq!(packet_info.packet, tc_slice);
    }

    #[test]
    fn test_accessing_combined_distributor() {
        let pus_handler = PusHandlerOwnedQueue::default();
        let handler_base = BasicApidHandlerOwnedQueue::default();
        let pus_distrib = PusDistributor::new(pus_handler);

        let apid_handler = ApidHandlerOwned {
            pus_distrib,
            handler_base,
        };
        let mut ccsds_distrib = CcsdsDistributor::new(apid_handler);

        let mut test_buf: [u8; 32] = [0; 32];
        let tc_slice = generate_ping_tc(test_buf.as_mut_slice());

        ccsds_distrib
            .pass_tc(tc_slice)
            .expect("Passing TC slice failed");

        let apid_handler_casted_back = ccsds_distrib.packet_handler_mut();
        assert!(!apid_handler_casted_back
            .handler_base
            .known_packet_queue
            .is_empty());
        let handler_owned_queue = apid_handler_casted_back
            .pus_distrib
            .service_distributor_mut();
        assert!(!handler_owned_queue.0.is_empty());
        let packet_info = handler_owned_queue.0.pop_front().unwrap();
        assert_eq!(packet_info.service, 17);
        assert_eq!(packet_info.apid, 0x002);
        assert_eq!(packet_info.packet, tc_slice);
    }

    #[test]
    fn test_pus_distrib_error_custom_error() {
        let error = PusDistribError::CustomError(GenericSendError::RxDisconnected);
        let error_string = format!("{}", error);
        assert_eq!(
            error_string,
            "pus distribution error: rx side has disconnected"
        );
    }

    #[test]
    fn test_pus_distrib_error_pus_error() {
        let error = PusDistribError::<GenericSendError>::PusError(PusError::CrcCalculationMissing);
        let error_string = format!("{}", error);
        assert_eq!(
            error_string,
            "pus distribution error: crc16 was not calculated"
        );
    }
}
