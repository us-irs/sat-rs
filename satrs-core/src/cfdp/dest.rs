use super::{State, TransactionStep};
use spacepackets::cfdp::{
    pdu::{CommonPduConfig, FileDirectiveType},
    PduType,
};

pub struct DestinationHandler {
    step: TransactionStep,
    state: State,
    //pdu_conf: CommonPduConfig,
}

impl DestinationHandler {
    pub fn new() -> Self {
        Self {
            step: TransactionStep::Idle,
            state: State::Idle,
            //pdu_conf: CommonPduConfig::new_with_defaults(),
        }
    }

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
            State::BusyClass1Nacked => self.fsm_nacked(),
            State::BusyClass2Acked => todo!(),
        }
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
