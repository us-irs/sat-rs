use spacepackets::cfdp::{pdu::FileDirectiveType, PduType};

use super::{State, TransactionStep};

pub struct DestinationHandler {
    step: TransactionStep,
    state: State,
}

impl DestinationHandler {
    pub fn insert_packet(
        &mut self,
        pdu_type: PduType,
        pdu_directive: Option<FileDirectiveType>,
        raw_packet: &[u8],
    ) {
    }
    pub fn state_machine(&mut self) {
        match self.state {
            State::Idle => todo!(),
            State::BusyClass1Nacked => todo!(),
            State::BusyClass2Acked => todo!(),
        }
    }
}
