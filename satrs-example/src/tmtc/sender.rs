use std::{cell::RefCell, collections::VecDeque, sync::mpsc};

use satrs::{
    pus::EcssTmSender,
    queue::GenericSendError,
    spacepackets::ecss::WritablePusPacket,
    tmtc::{PacketAsVec, PacketSenderRaw, PacketSenderWithSharedPool, StoreAndSendError},
    ComponentId,
};

#[derive(Default, Debug, Clone)]
pub struct MockSender(pub RefCell<VecDeque<PacketAsVec>>);

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum TmTcSender {
    Static(PacketSenderWithSharedPool),
    Heap(mpsc::SyncSender<PacketAsVec>),
    Mock(MockSender),
}

impl TmTcSender {
    #[allow(dead_code)]
    pub fn get_mock_sender(&mut self) -> Option<&mut MockSender> {
        match self {
            TmTcSender::Mock(sender) => Some(sender),
            _ => None,
        }
    }
}

impl EcssTmSender for TmTcSender {
    fn send_tm(
        &self,
        sender_id: satrs::ComponentId,
        tm: satrs::pus::PusTmVariant,
    ) -> Result<(), satrs::pus::EcssTmtcError> {
        match self {
            TmTcSender::Static(sync_sender) => sync_sender.send_tm(sender_id, tm),
            TmTcSender::Heap(sync_sender) => match tm {
                satrs::pus::PusTmVariant::InStore(_) => panic!("can not send TM in store"),
                satrs::pus::PusTmVariant::Direct(pus_tm_creator) => sync_sender
                    .send(PacketAsVec::new(sender_id, pus_tm_creator.to_vec()?))
                    .map_err(|_| GenericSendError::RxDisconnected.into()),
            },
            TmTcSender::Mock(_) => Ok(()),
        }
    }
}

impl PacketSenderRaw for TmTcSender {
    type Error = StoreAndSendError;

    fn send_packet(&self, sender_id: ComponentId, packet: &[u8]) -> Result<(), Self::Error> {
        match self {
            TmTcSender::Static(packet_sender_with_shared_pool) => {
                packet_sender_with_shared_pool.send_packet(sender_id, packet)
            }
            TmTcSender::Heap(sync_sender) => sync_sender
                .send_packet(sender_id, packet)
                .map_err(StoreAndSendError::Send),
            TmTcSender::Mock(sender) => sender.send_packet(sender_id, packet),
        }
    }
}

impl PacketSenderRaw for MockSender {
    type Error = StoreAndSendError;

    fn send_packet(&self, sender_id: ComponentId, tc_raw: &[u8]) -> Result<(), Self::Error> {
        let mut mut_queue = self.0.borrow_mut();
        mut_queue.push_back(PacketAsVec::new(sender_id, tc_raw.to_vec()));
        Ok(())
    }
}
