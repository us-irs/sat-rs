use super::{State, TransactionStep};
use spacepackets::cfdp::{
    pdu::{metadata::MetadataPdu, CommonPduConfig, FileDirectiveType, PduError},
    PduType,
};

pub struct DestinationHandler {
    step: TransactionStep,
    state: State,
    pdu_conf: CommonPduConfig,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum DestError {
    /// File directive expected, but none specified
    DirectiveExpected,
    CantProcessPacketType(FileDirectiveType),
    // Received new metadata PDU while being already being busy with a file transfer.
    RecvdMetadataButIsBusy,
    Pdu(PduError),
}

impl From<PduError> for DestError {
    fn from(value: PduError) -> Self {
        Self::Pdu(value)
    }
}

impl DestinationHandler {
    pub fn new() -> Self {
        Self {
            step: TransactionStep::Idle,
            state: State::Idle,
            pdu_conf: CommonPduConfig::new_with_defaults(),
        }
    }

    pub fn insert_packet(
        &mut self,
        pdu_type: PduType,
        pdu_directive: Option<FileDirectiveType>,
        raw_packet: &[u8],
    ) -> Result<(), DestError> {
        match pdu_type {
            PduType::FileDirective => {
                if pdu_directive.is_none() {
                    return Err(DestError::DirectiveExpected);
                }
                self.handle_file_directive(pdu_directive.unwrap(), raw_packet)
            }
            PduType::FileData => self.handle_file_data(raw_packet),
        }
    }

    pub fn handle_file_data(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
        Ok(())
    }

    pub fn handle_file_directive(
        &mut self,
        pdu_directive: FileDirectiveType,
        raw_packet: &[u8],
    ) -> Result<(), DestError> {
        match pdu_directive {
            FileDirectiveType::EofPdu => todo!(),
            FileDirectiveType::FinishedPdu => todo!(),
            FileDirectiveType::AckPdu => todo!(),
            FileDirectiveType::MetadataPdu => self.handle_metadata_pdu(raw_packet),
            FileDirectiveType::NakPdu => todo!(),
            FileDirectiveType::PromptPdu => todo!(),
            FileDirectiveType::KeepAlivePdu => todo!(),
        };
        Ok(())
    }

    pub fn state_machine(&mut self) {
        match self.state {
            State::Idle => todo!(),
            State::BusyClass1Nacked => self.fsm_nacked(),
            State::BusyClass2Acked => todo!(),
        }
    }

    pub fn handle_metadata_pdu(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
        if self.state != State::Idle {
            return Err(DestError::RecvdMetadataButIsBusy);
        }
        let metadata_pdu = MetadataPdu::from_bytes(raw_packet)?;
        let params = metadata_pdu.metadata_params();

        Ok(())
    }

    pub fn handle_eof_pdu(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
        Ok(())
    }

    fn fsm_nacked(&self) {
        match self.step {
            TransactionStep::Idle => {
                // TODO: Should not happen. Determine what to do later
            }
            TransactionStep::TransactionStart => {}
            TransactionStep::ReceivingFileDataPdus => todo!(),
            TransactionStep::SendingAckPdu => todo!(),
            TransactionStep::TransferCompletion => todo!(),
            TransactionStep::SendingFinishedPdu => todo!(),
        }
    }

    /// Get the step, which denotes the exact step of a pending CFDP transaction when applicable.
    pub fn step(&self) -> TransactionStep {
        self.step
    }

    /// Get the step, which denotes whether the CFDP handler is active, and which CFDP class
    /// is used if it is active.
    pub fn state(&self) -> State {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let dest_handler = DestinationHandler::new();
        assert_eq!(dest_handler.state(), State::Idle);
        assert_eq!(dest_handler.step(), TransactionStep::Idle);
    }
}
