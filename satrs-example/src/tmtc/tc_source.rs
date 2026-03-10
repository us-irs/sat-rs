use models::{ccsds::CcsdsTcPacketOwned, ComponentId, TcHeader};
use satrs::{
    pus::HandlingStatus,
    spacepackets::{CcsdsPacketReader, ChecksumType},
    tmtc::PacketAsVec,
};
use std::{
    collections::HashMap,
    sync::mpsc::{self, TryRecvError},
};

pub type CcsdsDistributor = HashMap<ComponentId, std::sync::mpsc::SyncSender<CcsdsTcPacketOwned>>;

// TC source components where the heap is the backing memory of the received telecommands.
pub struct TcSourceTask {
    pub tc_receiver: mpsc::Receiver<PacketAsVec>,
    ccsds_distributor: CcsdsDistributor,
}

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

    pub fn add_target(
        &mut self,
        target_id: ComponentId,
        sender: mpsc::SyncSender<CcsdsTcPacketOwned>,
    ) {
        self.ccsds_distributor.insert(target_id, sender);
    }

    pub fn periodic_operation(&mut self) {
        loop {
            if self.poll_tc() == HandlingStatus::Empty {
                break;
            }
        }
    }

    pub fn poll_tc(&mut self) -> HandlingStatus {
        match self.tc_receiver.try_recv() {
            Ok(packet) => {
                log::debug!("received raw packet: {:?}", packet);
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
                    log::debug!("sending TC packet to target ID: {:?}", tc_header.target_id);
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
