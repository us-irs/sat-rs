use satrs::pus::HandlingStatus;
use satrs_example::{CcsdsTcPacketOwned, ComponentId};
use std::{
    collections::HashMap,
    sync::mpsc::{self, TryRecvError},
};

// TC source components where static pools are the backing memory of the received telecommands.
/*
pub struct TcSourceTaskStatic {
    shared_tc_pool: SharedPacketPool,
    tc_receiver: mpsc::Receiver<PacketInPool>,
    /// We allocate this buffer from the heap to avoid a clippy warning on large enum variant
    /// differences.
    tc_buf: Box<[u8; 4096]>,
    pus_distributor: PusTcDistributor,
}

#[allow(dead_code)]
impl TcSourceTaskStatic {
    pub fn new(
        shared_tc_pool: SharedPacketPool,
        tc_receiver: mpsc::Receiver<PacketInPool>,
        pus_receiver: PusTcDistributor,
    ) -> Self {
        Self {
            shared_tc_pool,
            tc_receiver,
            tc_buf: Box::new([0; 4096]),
            pus_distributor: pus_receiver,
        }
    }

    pub fn periodic_operation(&mut self) {
        self.poll_tc();
    }

    pub fn poll_tc(&mut self) -> HandlingStatus {
        // Right now, we only expect ECSS PUS packets.
        // If packets like CFDP are expected, we might have to check the APID first.
        match self.tc_receiver.try_recv() {
            Ok(packet_in_pool) => {
                let pool = self
                    .shared_tc_pool
                    .0
                    .read()
                    .expect("locking tc pool failed");
                pool.read(&packet_in_pool.store_addr, self.tc_buf.as_mut_slice())
                    .expect("reading pool failed");
                drop(pool);
                self.pus_distributor
                    .handle_tc_packet_in_store(packet_in_pool, self.tc_buf.as_slice())
                    .ok();
                HandlingStatus::HandledOne
            }
            Err(e) => match e {
                TryRecvError::Empty => HandlingStatus::Empty,
                TryRecvError::Disconnected => {
                    log::warn!("tmtc thread: sender disconnected");
                    HandlingStatus::Empty
                }
            },
        }
    }
}
*/

pub type CcsdsDistributor = HashMap<ComponentId, std::sync::mpsc::SyncSender<CcsdsTcPacketOwned>>;

// TC source components where the heap is the backing memory of the received telecommands.
pub struct TcSourceTask {
    pub tc_receiver: mpsc::Receiver<CcsdsTcPacketOwned>,
    ccsds_distributor: CcsdsDistributor,
}

//#[allow(dead_code)]
impl TcSourceTask {
    pub fn new(
        tc_receiver: mpsc::Receiver<CcsdsTcPacketOwned>,
        ccsds_distributor: CcsdsDistributor,
    ) -> Self {
        Self {
            tc_receiver,
            ccsds_distributor,
        }
    }

    pub fn periodic_operation(&mut self) {
        self.poll_tc();
    }

    pub fn poll_tc(&mut self) -> HandlingStatus {
        // Right now, we only expect ECSS PUS packets.
        // If packets like CFDP are expected, we might have to check the APID first.
        match self.tc_receiver.try_recv() {
            Ok(packet) => {
                if let Some(sender) = self.ccsds_distributor.get(&packet.tc_header.target_id) {
                    sender.send(packet).ok();
                } else {
                    log::warn!(
                        "no TC handler for target ID {:?}",
                        packet.tc_header.target_id
                    );
                    // TODO: Send a dedicated TM packet.
                }
                HandlingStatus::HandledOne
            }
            Err(e) => match e {
                TryRecvError::Empty => HandlingStatus::Empty,
                TryRecvError::Disconnected => {
                    log::warn!("tmtc thread: sender disconnected");
                    HandlingStatus::Empty
                }
            },
        }
    }
}
