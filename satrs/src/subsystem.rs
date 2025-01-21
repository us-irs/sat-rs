use crate::{
    mode::{Mode, ModeAndSubmode, ModeReply, ModeRequest, ModeRequestSender, UNKNOWN_MODE},
    mode_tree::{
        ModeStoreProvider, ModeStoreVec, SequenceModeTables, SequenceTableMapTable,
        SequenceTablesMapValue, TargetModeTables, TargetNotInModeStoreError, TargetTablesMapValue,
    },
    queue::GenericTargetedMessagingError,
    request::{GenericMessage, RequestId},
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
pub enum ModeCommandingResult {
    CommandingDone,
    CommandingStepDone,
    AwaitingSuccessCheck,
}

#[derive(Debug, thiserror::Error)]
#[error("Mode {0} does not exist")]
pub struct ModeDoesNotExistError(Mode);

#[derive(Debug)]
pub struct SequenceExecutionHelper {
    target_mode: Option<Mode>,
    state: SequenceExecutionHelperStates,
    request_id: RequestId,
    current_sequence_index: Option<usize>,
}

impl Default for SequenceExecutionHelper {
    fn default() -> Self {
        Self {
            target_mode: None,
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
        self.target_mode = Some(mode);
        self.request_id = request_id;
        self.current_sequence_index = None;
        Ok(())
    }

    pub fn target_mode(&self) -> Option<Mode> {
        self.target_mode
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
    ) -> Result<ModeCommandingResult, GenericTargetedMessagingError> {
        if self.state == SequenceExecutionHelperStates::AwaitingCheckSuccess {
            return Ok(ModeCommandingResult::AwaitingSuccessCheck);
        }
        if self.target_mode.is_none() {
            return Ok(ModeCommandingResult::CommandingDone);
        }
        match self.current_sequence_index {
            Some(idx) => {
                // Execute the sequence.
                let seq_table_value = table.0.get(&self.target_mode.unwrap()).unwrap();
                self.execute_sequence_and_map_to_result(
                    seq_table_value,
                    idx,
                    sender,
                    mode_store_vec,
                )
            }
            None => {
                // Find the first sequence
                let seq_table_value = table.0.get(&self.target_mode.unwrap()).unwrap();
                if seq_table_value.entries.is_empty() {
                    Ok(ModeCommandingResult::CommandingDone)
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
    ) -> Result<ModeCommandingResult, GenericTargetedMessagingError> {
        if Self::execute_sequence(
            self.request_id,
            &seq_table_value.entries[sequence_idx],
            sender,
            mode_store_vec,
        )? {
            self.state = SequenceExecutionHelperStates::AwaitingCheckSuccess;
            Ok(ModeCommandingResult::AwaitingSuccessCheck)
        } else if seq_table_value.entries.len() - 1 == sequence_idx {
            return Ok(ModeCommandingResult::CommandingDone);
        } else {
            self.current_sequence_index = Some(sequence_idx + 1);
            return Ok(ModeCommandingResult::CommandingStepDone);
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

#[derive(Debug, Default, PartialEq, Eq)]
pub enum ModeTreeHelperState {
    #[default]
    Idle,
    TargetKeeping = 1,
    ModeCommanding = 2,
}

#[derive(Debug, Default)]
pub enum SubsystemHelperResult {
    #[default]
    Idle,
    TargetKeeping(TargetKeepingResult),
    ModeCommanding(ModeCommandingResult),
}

impl From<TargetKeepingResult> for SubsystemHelperResult {
    fn from(value: TargetKeepingResult) -> Self {
        Self::TargetKeeping(value)
    }
}

impl From<ModeCommandingResult> for SubsystemHelperResult {
    fn from(value: ModeCommandingResult) -> Self {
        Self::ModeCommanding(value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModeTreeHelperError {
    #[error("generic targeted messaging error: {0}")]
    Message(#[from] GenericTargetedMessagingError),
    #[error("current mode {0} is not contained in target table")]
    CurrentModeNotInTargetTable(Mode),
}

pub struct SubsystemCommandingHelper {
    pub current_mode: ModeAndSubmode,
    pub state: ModeTreeHelperState,
    pub children_mode_store: ModeStoreVec,
    pub active_request_id: Option<RequestId>,
    pub target_tables: TargetModeTables,
    pub sequence_tables: SequenceModeTables,
    pub seq_exec_helper: SequenceExecutionHelper,
}

impl Default for SubsystemCommandingHelper {
    fn default() -> Self {
        Self {
            current_mode: UNKNOWN_MODE,
            state: Default::default(),
            children_mode_store: Default::default(),
            active_request_id: None,
            target_tables: Default::default(),
            sequence_tables: Default::default(),
            seq_exec_helper: Default::default(),
        }
    }
}

impl SubsystemCommandingHelper {
    pub fn new(
        children_mode_store: ModeStoreVec,
        target_tables: TargetModeTables,
        sequence_tables: SequenceModeTables,
    ) -> Self {
        Self {
            current_mode: UNKNOWN_MODE,
            state: ModeTreeHelperState::Idle,
            children_mode_store,
            active_request_id: None,
            target_tables,
            sequence_tables,
            seq_exec_helper: Default::default(),
        }
    }

    pub fn add_mode_child(&mut self, child: ComponentId, mode: ModeAndSubmode) {
        self.children_mode_store.add_component(child, mode);
    }

    pub fn add_target_and_sequence_table(
        &mut self,
        mode: Mode,
        target_table_val: TargetTablesMapValue,
        sequence_table_val: SequenceTablesMapValue,
    ) {
        self.target_tables.0.insert(mode, target_table_val);
        self.sequence_tables.0.insert(mode, sequence_table_val);
    }

    pub fn start_command_sequence(
        &mut self,
        mode: Mode,
        request_id: RequestId,
    ) -> Result<(), ModeDoesNotExistError> {
        self.seq_exec_helper.load(mode, request_id, &self.sequence_tables)?;
        self.state = ModeTreeHelperState::ModeCommanding;
        Ok(())
    }

    pub fn send_announce_mode_cmd_to_children(
        &self,
        request_id: RequestId,
        req_sender: &impl ModeRequestSender,
        recursive: bool,
    ) -> Result<(), GenericTargetedMessagingError> {
        let mut request = ModeRequest::AnnounceMode;
        if recursive {
            request = ModeRequest::AnnounceModeRecursive;
        }
        for child in &self.children_mode_store.0 {
            req_sender.send_mode_request(request_id, child.id(), request)?;
        }
        Ok(())
    }

    pub fn state_machine(
        &mut self,
        opt_reply: Option<GenericMessage<ModeReply>>,
        req_sender: &impl ModeRequestSender,
    ) -> Result<SubsystemHelperResult, ModeTreeHelperError> {
        if let Some(reply) = opt_reply {
            self.handle_mode_reply(&reply);
        }
        match self.state {
            ModeTreeHelperState::Idle => Ok(SubsystemHelperResult::Idle),
            ModeTreeHelperState::TargetKeeping => {
                // We check whether the current mode is modelled by a target table first.
                if let Some(target_table) = self.target_tables.0.get(&self.current_mode.mode()) {
                    return Ok(self.perform_target_keeping(target_table).into());
                }
                Ok(TargetKeepingResult::Ok.into())
            }
            ModeTreeHelperState::ModeCommanding => {
                let result = self.seq_exec_helper.run(
                    &self.sequence_tables,
                    req_sender,
                    &mut self.children_mode_store,
                )?;
                // By default, the helper will automatically transition into the target keeping
                // mode after an executed sequence.
                if let ModeCommandingResult::CommandingDone = result {
                    self.state = ModeTreeHelperState::TargetKeeping;
                    self.active_request_id = None;
                    self.current_mode = ModeAndSubmode::new(self.seq_exec_helper.target_mode().unwrap(), 0);
                }
                Ok(result.into())
            }
        }
    }

    pub fn perform_target_keeping(
        &self,
        target_table: &TargetTablesMapValue,
    ) -> TargetKeepingResult {
        for entry in &target_table.entries {
            if !entry.monitor_state {
                continue;
            }
            let mut target_mode_violated = false;
            self.children_mode_store.0.iter().for_each(|val| {
                if val.id() == entry.common.target_id {
                    target_mode_violated =
                        if let Some(allowed_submode_mask) = entry.allowed_submode_mask() {
                            let fixed_bits = !allowed_submode_mask;
                            (val.mode_and_submode().mode() != entry.common.mode_submode.mode())
                                && (val.mode_and_submode().submode() & fixed_bits
                                    != entry.common.mode_submode.submode() & fixed_bits)
                        } else {
                            val.mode_and_submode() != entry.common.mode_submode
                        };
                }
            });
            if target_mode_violated {
                // Target keeping violated. Report violation and fallback mode to user.
                return TargetKeepingResult::Violated {
                    fallback_mode: target_table.fallback_mode,
                };
            }
        }
        TargetKeepingResult::Ok
    }

    pub fn handle_mode_reply(&mut self, reply: &GenericMessage<ModeReply>) {
        if !self.children_mode_store.has_component(reply.sender_id()) {
            return;
        }
        let mut generic_mode_reply_handler =
            |sender_id, mode_and_submode: Option<ModeAndSubmode>| {
                // Tying the reply awaition to the request ID ensures that something like replies
                // belonging to older requests do not interfere with the completion handling of
                // the mode commanding. This is important for forced mode commands.
                let mut handle_awaition = false;
                if self.state == ModeTreeHelperState::ModeCommanding
                    && self.active_request_id.is_some()
                    && reply.request_id() == self.active_request_id.unwrap()
                {
                    handle_awaition = true;
                }
                let still_awating_replies = self.children_mode_store.generic_reply_handler(
                    sender_id,
                    mode_and_submode,
                    handle_awaition,
                );
                if self.state == ModeTreeHelperState::ModeCommanding && !still_awating_replies {
                    self.seq_exec_helper.confirm_sequence_done();
                }
            };
        match reply.message {
            ModeReply::ModeInfo(mode_and_submode) => {
                generic_mode_reply_handler(reply.sender_id(), Some(mode_and_submode));
            }
            ModeReply::ModeReply(mode_and_submode) => {
                generic_mode_reply_handler(reply.sender_id(), Some(mode_and_submode));
            }
            ModeReply::CantReachMode(_) => {
                generic_mode_reply_handler(reply.sender_id(), None);
            }
            ModeReply::WrongMode { reached, .. } => {
                generic_mode_reply_handler(reply.sender_id(), Some(reached));
            }
        };
    }

    pub fn update_child_mode(
        &mut self,
        child: ComponentId,
        mode: ModeAndSubmode,
    ) -> Result<(), TargetNotInModeStoreError> {
        self.children_mode_store.set_mode(child, mode)
    }
}

#[cfg(test)]
mod tests {
    // TODO: Test sequence execution helper
    // TODO: Test subsystem commanding helper
}
