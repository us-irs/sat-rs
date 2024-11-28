use crate::{
    mode::{Mode, ModeAndSubmode, ModeRequest, ModeRequestSender},
    mode_tree::{SequenceModeTables, SequenceTableMapTable, SequenceTablesMapValue},
    queue::GenericTargetedMessagingError,
    request::RequestId,
    ComponentId,
};

#[derive(Debug, PartialEq, Eq)]
pub enum SequenceExecutionHelperStates {
    Idle,
    AwaitingCheckSuccess,
    Done,
}

#[derive(Debug)]
pub struct SequenceExecutionHelper {
    target_mode: Mode,
    state: SequenceExecutionHelperStates,
    request_id: RequestId,
    current_sequence_index: Option<usize>,
}

pub trait CheckSuccessProvider {
    fn mode_request_requires_success_check(
        &mut self,
        target_id: ComponentId,
        target_mode: ModeAndSubmode,
    );
}

#[derive(Debug)]
pub enum SequenceHandlerResult {
    SequenceDone,
    SequenceStepDone,
    AwaitingSuccessCheck,
}

impl SequenceExecutionHelper {
    pub fn new(
        mode: Mode,
        request_id: RequestId,
        sequence_tables: &SequenceModeTables,
    ) -> Option<Self> {
        if !sequence_tables.0.contains_key(&mode) {
            return None;
        }
        Some(Self {
            target_mode: mode,
            state: SequenceExecutionHelperStates::Idle,
            request_id,
            current_sequence_index: None,
        })
    }

    pub fn confirm_sequence_done(&mut self) {
        if let SequenceExecutionHelperStates::AwaitingCheckSuccess = self.state {
            self.state = SequenceExecutionHelperStates::Idle;
        }
    }

    pub fn run(
        &mut self,
        table: &SequenceModeTables,
        sender: &impl ModeRequestSender,
    ) -> Result<SequenceHandlerResult, GenericTargetedMessagingError> {
        if self.state == SequenceExecutionHelperStates::AwaitingCheckSuccess {
            return Ok(SequenceHandlerResult::AwaitingSuccessCheck);
        }
        match self.current_sequence_index {
            Some(idx) => {
                // Execute the sequence.
                let seq_table_value = table.0.get(&self.target_mode).unwrap();
                self.execute_sequence_and_map_to_result(seq_table_value, idx, sender)
            }
            None => {
                // Find the first sequence
                let seq_table_value = table.0.get(&self.target_mode).unwrap();
                if seq_table_value.entries.is_empty() {
                    Ok(SequenceHandlerResult::SequenceDone)
                } else {
                    self.current_sequence_index = Some(0);
                    self.execute_sequence_and_map_to_result(seq_table_value, 0, sender)
                }
            }
        }
    }

    pub fn execute_sequence_and_map_to_result(
        &mut self,
        seq_table_value: &SequenceTablesMapValue,
        sequence_idx: usize,
        sender: &impl ModeRequestSender,
    ) -> Result<SequenceHandlerResult, GenericTargetedMessagingError> {
        if Self::execute_sequence(
            self.request_id,
            &seq_table_value.entries[sequence_idx],
            sender,
        )? {
            self.state = SequenceExecutionHelperStates::AwaitingCheckSuccess;
            Ok(SequenceHandlerResult::AwaitingSuccessCheck)
        } else if seq_table_value.entries.len() - 1 == sequence_idx {
            return Ok(SequenceHandlerResult::SequenceDone);
        } else {
            self.current_sequence_index = Some(sequence_idx + 1);
            return Ok(SequenceHandlerResult::SequenceStepDone);
        }
    }

    pub fn execute_sequence(
        request_id: RequestId,
        map_table: &SequenceTableMapTable,
        sender: &impl ModeRequestSender,
    ) -> Result<bool, GenericTargetedMessagingError> {
        let mut some_succes_check_required = false;
        for entry in &map_table.entries {
            sender.send_mode_request(
                request_id,
                entry.common.target_id,
                ModeRequest::SetMode(entry.common.mode_submode),
            )?;
            if entry.check_success {
                some_succes_check_required = true;
            }
        }
        Ok(some_succes_check_required)
    }
}

#[cfg(test)]
mod tests {}
