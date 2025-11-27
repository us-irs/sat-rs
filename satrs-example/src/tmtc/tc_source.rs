use satrs::{
    pus::HandlingStatus,
    spacepackets::{CcsdsPacketReader, ChecksumType},
    tmtc::PacketAsVec,
};
use satrs_example::{CcsdsTcPacketOwned, ComponentId, TcHeader};
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
    pub tc_receiver: mpsc::Receiver<PacketAsVec>,
    ccsds_distributor: CcsdsDistributor,
}

//#[allow(dead_code)]
impl TcSourceTask {
    pub fn new(
        tc_receiver: mpsc::Receiver<PacketAsVec>,
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
                let ccsds_tc_reader_result =
                    CcsdsPacketReader::new(&packet.packet, Some(ChecksumType::WithCrc16));
                if ccsds_tc_reader_result.is_err() {
                    log::warn!(
                        "received invalid CCSDS TC packet: {:?}",
                        ccsds_tc_reader_result.err()
                    );
                    // TODO: Send a dedicated TM packet.
                    return HandlingStatus::HandledOne;
                }
                let ccsds_tc_reader = ccsds_tc_reader_result.unwrap();
                let tc_header_result =
                    postcard::take_from_bytes::<TcHeader>(ccsds_tc_reader.user_data());
                if tc_header_result.is_err() {
                    log::warn!(
                        "received CCSDS TC packet with invalid TC header: {:?}",
                        tc_header_result.err()
                    );
                    // TODO: Send a dedicated TM packet.
                    return HandlingStatus::HandledOne;
                }
                let (tc_header, payload) = tc_header_result.unwrap();
                if let Some(sender) = self.ccsds_distributor.get(&tc_header.target_id) {
                    sender
                        .send(CcsdsTcPacketOwned {
                            sp_header: *ccsds_tc_reader.sp_header(),
                            tc_header,
                            payload: payload.to_vec(),
                        })
                        .ok();
                } else {
                    log::warn!("no TC handler for target ID {:?}", tc_header.target_id);
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
