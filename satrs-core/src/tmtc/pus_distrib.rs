//! ECSS PUS packet routing components.
//!
//! The routing components consist of two core components:
//!  1. [PusDistributor] component which dispatches received packets to a user-provided handler.
//!  2. [PusServiceProvider] trait which should be implemented by the user-provided PUS packet
//!     handler.
//!
//! The [PusDistributor] implements the [ReceivesEcssPusTc], [ReceivesCcsdsTc] and the
//! [ReceivesTcCore] trait which allows to pass raw packets, CCSDS packets and PUS TC packets into
//! it. Upon receiving a packet, it performs the following steps:
//!
//! 1. It tries to extract the [SpHeader] and [spacepackets::ecss::tc::PusTc] objects from the raw
//!    bytestream. If this process fails, a [PusDistribError::PusError] is returned to the user.
//! 2. If it was possible to extract both components, the packet will be passed to the
//!    [PusServiceProvider::handle_pus_tc_packet] method provided by the user.
//!
//! # Example
//!
//! ```rust
//! use spacepackets::ecss::SerializablePusPacket;
//! use satrs_core::tmtc::pus_distrib::{PusDistributor, PusServiceProvider};
//! use satrs_core::tmtc::{ReceivesTc, ReceivesTcCore};
//! use spacepackets::SpHeader;
//! use spacepackets::ecss::tc::{PusTcCreator, PusTcReader};
//! struct ConcretePusHandler {
//!     handler_call_count: u32
//! }
//!
//! // This is a very simple possible service provider. It increments an internal call count field,
//! // which is used to verify the handler was called
//! impl PusServiceProvider for ConcretePusHandler {
//!     type Error = ();
//!     fn handle_pus_tc_packet(&mut self, service: u8, header: &SpHeader, pus_tc: &PusTcReader) -> Result<(), Self::Error> {
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
//! // Create and pass PUS ping telecommand with a valid APID
//! let mut space_packet_header = SpHeader::tc_unseg(0x002, 0x34, 0).unwrap();
//! let mut pus_tc = PusTcCreator::new_simple(&mut space_packet_header, 17, 1, None, true);
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
//! let concrete_handler_ref: &ConcretePusHandler = pus_distributor
//!     .service_provider_ref()
//!     .expect("Casting back to concrete type failed");
//! assert_eq!(concrete_handler_ref.handler_call_count, 1);
//! ```
use crate::pus::ReceivesEcssPusTc;
use crate::tmtc::{ReceivesCcsdsTc, ReceivesTcCore};
use alloc::boxed::Box;
use core::fmt::{Display, Formatter};
use downcast_rs::Downcast;
use spacepackets::ecss::tc::PusTcReader;
use spacepackets::ecss::{PusError, PusPacket};
use spacepackets::SpHeader;
#[cfg(feature = "std")]
use std::error::Error;

pub trait PusServiceProvider: Downcast {
    type Error;
    fn handle_pus_tc_packet(
        &mut self,
        service: u8,
        header: &SpHeader,
        pus_tc: &PusTcReader,
    ) -> Result<(), Self::Error>;
}
downcast_rs::impl_downcast!(PusServiceProvider assoc Error);

pub trait SendablePusServiceProvider: PusServiceProvider + Send {}

impl<T: Send + PusServiceProvider> SendablePusServiceProvider for T {}

downcast_rs::impl_downcast!(SendablePusServiceProvider assoc Error);

/// Generic distributor object which dispatches received packets to a user provided handler.
///
/// This distributor expects the passed trait object to be [Send]able to allow more ergonomic
/// usage with threads.
pub struct PusDistributor<E> {
    pub service_provider: Box<dyn SendablePusServiceProvider<Error = E>>,
}

impl<E> PusDistributor<E> {
    pub fn new(service_provider: Box<dyn SendablePusServiceProvider<Error = E>>) -> Self {
        PusDistributor { service_provider }
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
            PusDistribError::CustomError(e) => write!(f, "{e}"),
            PusDistribError::PusError(e) => write!(f, "{e}"),
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

impl<E: 'static> ReceivesTcCore for PusDistributor<E> {
    type Error = PusDistribError<E>;
    fn pass_tc(&mut self, tm_raw: &[u8]) -> Result<(), Self::Error> {
        // Convert to ccsds and call pass_ccsds
        let (sp_header, _) = SpHeader::from_be_bytes(tm_raw)
            .map_err(|e| PusDistribError::PusError(PusError::ByteConversion(e)))?;
        self.pass_ccsds(&sp_header, tm_raw)
    }
}

impl<E: 'static> ReceivesCcsdsTc for PusDistributor<E> {
    type Error = PusDistribError<E>;
    fn pass_ccsds(&mut self, header: &SpHeader, tm_raw: &[u8]) -> Result<(), Self::Error> {
        let (tc, _) = PusTcReader::new(tm_raw).map_err(|e| PusDistribError::PusError(e))?;
        self.pass_pus_tc(header, &tc)
    }
}

impl<E: 'static> ReceivesEcssPusTc for PusDistributor<E> {
    type Error = PusDistribError<E>;
    fn pass_pus_tc(&mut self, header: &SpHeader, pus_tc: &PusTcReader) -> Result<(), Self::Error> {
        self.service_provider
            .handle_pus_tc_packet(pus_tc.service(), header, pus_tc)
            .map_err(|e| PusDistribError::CustomError(e))
    }
}

impl<E: 'static> PusDistributor<E> {
    pub fn service_provider_ref<T: SendablePusServiceProvider<Error = E>>(&self) -> Option<&T> {
        self.service_provider.downcast_ref::<T>()
    }

    pub fn service_provider_mut<T: SendablePusServiceProvider<Error = E>>(
        &mut self,
    ) -> Option<&mut T> {
        self.service_provider.downcast_mut::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmtc::ccsds_distrib::tests::{
        generate_ping_tc, BasicApidHandlerOwnedQueue, BasicApidHandlerSharedQueue,
    };
    use crate::tmtc::ccsds_distrib::{CcsdsDistributor, CcsdsPacketHandler};
    use alloc::vec::Vec;
    use spacepackets::ecss::PusError;
    use spacepackets::CcsdsPacket;
    #[cfg(feature = "std")]
    use std::collections::VecDeque;
    #[cfg(feature = "std")]
    use std::sync::{Arc, Mutex};

    fn is_send<T: Send>(_: &T) {}

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
            pus_tc: &PusTcReader,
        ) -> Result<(), Self::Error> {
            let mut vec: Vec<u8> = Vec::new();
            vec.extend_from_slice(pus_tc.raw_data());
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
            pus_tc: &PusTcReader,
        ) -> Result<(), Self::Error> {
            let mut vec: Vec<u8> = Vec::new();
            vec.extend_from_slice(pus_tc.raw_data());
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

    impl CcsdsPacketHandler for ApidHandlerOwned {
        apid_handler_impl!();
    }

    impl CcsdsPacketHandler for ApidHandlerShared {
        apid_handler_impl!();
    }

    #[test]
    #[cfg(feature = "std")]
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

        let pus_distrib = PusDistributor {
            service_provider: Box::new(pus_handler),
        };
        is_send(&pus_distrib);
        let apid_handler = ApidHandlerShared {
            pus_distrib,
            handler_base,
        };
        let mut ccsds_distrib = CcsdsDistributor::new(Box::new(apid_handler));
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
        let pus_distrib = PusDistributor {
            service_provider: Box::new(pus_handler),
        };

        let apid_handler = ApidHandlerOwned {
            pus_distrib,
            handler_base,
        };
        let mut ccsds_distrib = CcsdsDistributor::new(Box::new(apid_handler));

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
