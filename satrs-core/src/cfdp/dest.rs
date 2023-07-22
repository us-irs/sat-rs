use super::{State, TransactionStep};
use spacepackets::cfdp::{pdu::FileDirectiveType, PduType};

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
    ) -> Result<(), ()> {
        match pdu_type {
            PduType::FileDirective => {
                if pdu_directive.is_none() {
                    return Err(());
                }
                self.handle_file_directive(pdu_directive.unwrap(), raw_packet)
            }
            PduType::FileData => self.handle_file_data(raw_packet),
        }
    }

    pub fn handle_file_data(&mut self, raw_packet: &[u8]) -> Result<(), ()> {
        Ok(())
    }
    pub fn handle_file_directive(
        &mut self,
        pdu_directive: FileDirectiveType,
        raw_packet: &[u8],
    ) -> Result<(), ()> {
        Ok(())
    }

    pub fn state_machine(&mut self) {
        match self.state {
            State::Idle => todo!(),
            State::BusyClass1Nacked => todo!(),
            State::BusyClass2Acked => todo!(),
        }
    }
}
