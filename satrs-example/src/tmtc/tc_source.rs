use satrs::{
    pool::PoolProvider,
    pus::HandlingStatus,
    tmtc::{PacketAsVec, PacketInPool, SharedPacketPool},
    ComponentId,
};
use std::{
    collections::HashMap,
    sync::mpsc::{self, TryRecvError},
};

use crate::pus::PusTcDistributor;

// TC source components where static pools are the backing memory of the received telecommands.
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

pub type CcsdsDistributorDyn = HashMap<ComponentId, std::sync::mpsc::SyncSender<PacketAsVec>>;
pub type CcsdsDistributorStatic = HashMap<ComponentId, std::sync::mpsc::SyncSender<PacketInPool>>;

// TC source components where the heap is the backing memory of the received telecommands.
pub struct TcSourceTaskDynamic {
    pub tc_receiver: mpsc::Receiver<PacketAsVec>,
    ccsds_distributor: CcsdsDistributorDyn,
}

#[allow(dead_code)]
impl TcSourceTaskDynamic {
    pub fn new(
        tc_receiver: mpsc::Receiver<PacketAsVec>,
        ccsds_distributor: CcsdsDistributorDyn,
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
            Ok(packet_as_vec) => {
                self.pus_distributor
                    .handle_tc_packet_vec(packet_as_vec)
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

#[allow(dead_code)]
pub enum TcSourceTask {
    Static(TcSourceTaskStatic),
    Heap(TcSourceTaskDynamic),
}

impl TcSourceTask {
    pub fn periodic_operation(&mut self) {
        match self {
            TcSourceTask::Static(task) => task.periodic_operation(),
            TcSourceTask::Heap(task) => task.periodic_operation(),
        }
    }
}
