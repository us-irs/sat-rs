use crate::{
    mode::{Mode, ModeAndSubmode, ModeRequest, ModeRequestSender},
    mode_tree::{ModeStoreVec, SequenceModeTables, SequenceTableMapTable, SequenceTablesMapValue},
    queue::GenericTargetedMessagingError,
    request::RequestId,
    ComponentId,
};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum SequenceExecutionHelperStates {
    Idle,
    AwaitingCheckSuccess,
    Done,
}

pub trait CheckSuccessProvider {
    fn mode_request_requires_success_check(
        &mut self,
        target_id: ComponentId,
        target_mode: ModeAndSubmode,
    );
}

#[derive(Debug)]
pub enum TargetKeepingResult {
    Ok,
    Violated { fallback_mode: Option<Mode> },
}

#[derive(Debug)]
pub enum SequenceHandlerResult {
    SequenceDone,
    SequenceStepDone,
    AwaitingSuccessCheck,
}

#[derive(Debug, thiserror::Error)]
#[error("Mode {0} does not exist")]
pub struct ModeDoesNotExistError(Mode);

#[derive(Debug)]
pub struct SequenceExecutionHelper {
    target_mode: Mode,
    state: SequenceExecutionHelperStates,
    request_id: RequestId,
    current_sequence_index: Option<usize>,
}

impl Default for SequenceExecutionHelper {
    fn default() -> Self {
        Self {
            target_mode: 0,
            state: SequenceExecutionHelperStates::Idle,
            request_id: 0,
            current_sequence_index: None,
        }
    }
}

impl SequenceExecutionHelper {
    pub fn load(
        &mut self,
        mode: Mode,
        request_id: RequestId,
        sequence_tables: &SequenceModeTables,
    ) -> Result<(), ModeDoesNotExistError> {
        if !sequence_tables.0.contains_key(&mode) {
            return Err(ModeDoesNotExistError(mode));
        }
        self.target_mode = mode;
        self.request_id = request_id;
        self.current_sequence_index = None;
        Ok(())
    }

    pub fn confirm_sequence_done(&mut self) {
        if let SequenceExecutionHelperStates::AwaitingCheckSuccess = self.state {
            self.state = SequenceExecutionHelperStates::Idle;
        }
    }

    pub fn state(&self) -> SequenceExecutionHelperStates {
        self.state
    }

    pub fn awaiting_check_success(&self) -> bool {
        matches!(
            self.state,
            SequenceExecutionHelperStates::AwaitingCheckSuccess
        )
    }

    pub fn current_sequence_index(&self) -> Option<usize> {
        self.current_sequence_index
    }

    pub fn run(
        &mut self,
        table: &SequenceModeTables,
        sender: &impl ModeRequestSender,
        mode_store_vec: &mut ModeStoreVec,
    ) -> Result<SequenceHandlerResult, GenericTargetedMessagingError> {
        if self.state == SequenceExecutionHelperStates::AwaitingCheckSuccess {
            return Ok(SequenceHandlerResult::AwaitingSuccessCheck);
        }
        match self.current_sequence_index {
            Some(idx) => {
                // Execute the sequence.
                let seq_table_value = table.0.get(&self.target_mode).unwrap();
                self.execute_sequence_and_map_to_result(
                    seq_table_value,
                    idx,
                    sender,
                    mode_store_vec,
                )
            }
            None => {
                // Find the first sequence
                let seq_table_value = table.0.get(&self.target_mode).unwrap();
                if seq_table_value.entries.is_empty() {
                    Ok(SequenceHandlerResult::SequenceDone)
                } else {
                    self.current_sequence_index = Some(0);
                    self.execute_sequence_and_map_to_result(
                        seq_table_value,
                        0,
                        sender,
                        mode_store_vec,
                    )
                }
            }
        }
    }

    pub fn execute_sequence_and_map_to_result(
        &mut self,
        seq_table_value: &SequenceTablesMapValue,
        sequence_idx: usize,
        sender: &impl ModeRequestSender,
        mode_store_vec: &mut ModeStoreVec,
    ) -> Result<SequenceHandlerResult, GenericTargetedMessagingError> {
        if Self::execute_sequence(
            self.request_id,
            &seq_table_value.entries[sequence_idx],
            sender,
            mode_store_vec,
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
        mode_store_vec: &mut ModeStoreVec,
    ) -> Result<bool, GenericTargetedMessagingError> {
        let mut some_succes_check_required = false;
        for entry in &map_table.entries {
            sender.send_mode_request(
                request_id,
                entry.common.target_id,
                ModeRequest::SetMode {
                    mode_and_submode: entry.common.mode_submode,
                    forced: false,
                },
            )?;
            mode_store_vec.0.iter_mut().for_each(|val| {
                if val.id() == entry.common.target_id {
                    val.awaiting_reply = true;
                }
            });
            if entry.check_success {
                some_succes_check_required = true;
            }
        }
        Ok(some_succes_check_required)
    }
}

#[cfg(test)]
mod tests {}
