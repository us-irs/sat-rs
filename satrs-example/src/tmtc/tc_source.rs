use satrs::{
    pool::PoolProvider,
    tmtc::{PacketAsVec, PacketInPool, PacketSenderWithSharedPool, SharedPacketPool},
};
use std::sync::mpsc::{self, TryRecvError};

use satrs::pus::MpscTmAsVecSender;

use crate::pus::{HandlingStatus, PusTcDistributor};

// TC source components where static pools are the backing memory of the received telecommands.
pub struct TcSourceTaskStatic {
    shared_tc_pool: SharedPacketPool,
    tc_receiver: mpsc::Receiver<PacketInPool>,
    tc_buf: [u8; 4096],
    pus_distributor: PusTcDistributor<PacketSenderWithSharedPool>,
}

impl TcSourceTaskStatic {
    pub fn new(
        shared_tc_pool: SharedPacketPool,
        tc_receiver: mpsc::Receiver<PacketInPool>,
        pus_receiver: PusTcDistributor<PacketSenderWithSharedPool>,
    ) -> Self {
        Self {
            shared_tc_pool,
            tc_receiver,
            tc_buf: [0; 4096],
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
                pool.read(&packet_in_pool.store_addr, &mut self.tc_buf)
                    .expect("reading pool failed");
                drop(pool);
                self.pus_distributor
                    .handle_tc_packet_in_store(packet_in_pool, &self.tc_buf)
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

// TC source components where the heap is the backing memory of the received telecommands.
pub struct TcSourceTaskDynamic {
    pub tc_receiver: mpsc::Receiver<PacketAsVec>,
    pus_distributor: PusTcDistributor<MpscTmAsVecSender>,
}

impl TcSourceTaskDynamic {
    pub fn new(
        tc_receiver: mpsc::Receiver<PacketAsVec>,
        pus_receiver: PusTcDistributor<MpscTmAsVecSender>,
    ) -> Self {
        Self {
            tc_receiver,
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
