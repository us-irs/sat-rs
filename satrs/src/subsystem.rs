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
pub enum SequenceExecutionHelperState {
    /// The sequence execution is IDLE, no command is loaded or the sequence exection has
    /// finished
    Idle,
    /// The sequence helper is executing a sequence and no replies need to be awaited.
    Busy,
    /// The sequence helper is still awaiting a reply from a mode children. The reply awaition
    /// is a property of a mode commanding sequence
    AwaitingCheckSuccess,
}

#[derive(Debug)]
pub enum TargetKeepingResult {
    Ok,
    Violated { fallback_mode: Option<Mode> },
}

#[derive(Debug)]
pub enum ModeCommandingResult {
    /// The commanding of all children is finished
    CommandingDone,
    /// One step of a commanding chain is finished
    CommandingStepDone,
    /// Reply awaition is required for some children
    AwaitingSuccessCheck,
}

#[derive(Debug, thiserror::Error)]
#[error("Mode {0} does not exist")]
pub struct ModeDoesNotExistError(Mode);

/// This sequence execution helper includes some boilerplate logic to
/// execute [SequenceModeTables].
///
/// It takes care of commanding the [ModeRequest]s specified in those tables and also includes the
/// states required to track the current progress of a sequence execution and take care of
/// reply and success awaition.
#[derive(Debug)]
pub struct SequenceExecutionHelper {
    target_mode: Option<Mode>,
    state: SequenceExecutionHelperState,
    request_id: Option<RequestId>,
    current_sequence_index: Option<usize>,
}

impl Default for SequenceExecutionHelper {
    fn default() -> Self {
        Self {
            target_mode: None,
            state: SequenceExecutionHelperState::Idle,
            request_id: None,
            current_sequence_index: None,
        }
    }
}

impl SequenceExecutionHelper {
    pub fn new() -> Self {
        Default::default()
    }

    /// Load a new mode sequence to be executed
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
        self.request_id = Some(request_id);
        self.state = SequenceExecutionHelperState::Busy;
        self.current_sequence_index = None;
        Ok(())
    }

    /// Run the sequence execution helper.
    ///
    /// This function will execute the sequence in the given [SequenceModeTables] based on the
    /// mode loaded in [Self::load]. It calls [Self::execute_sequence_and_map_to_result] and
    /// automatically takes care of state management, including increments of the sequence table
    /// index.
    ///
    /// The returnvalues of the helper have the following meaning.
    ///
    /// * [ModeCommandingResult::AwaitingSuccessCheck] - The sequence is still awaiting a success.
    ///   The user should check whether all children have reached the commanded target mode, for
    ///   example by checking [mode replies][ModeReply] received by the children components, and
    ///   then calling [Self::confirm_sequence_done] to advance to the sequence or complete the
    ///   sequence.
    /// * [ModeCommandingResult::CommandingDone] - The sequence is done. The user can load a new
    ///   sequence now without overwriting the last one. The sequence executor is in
    ///   [SequenceExecutionHelperState::Idle] again.
    /// * [ModeCommandingResult::CommandingStepDone] - The sequence has advanced one step. The user
    ///   can now call [Self::run] again to immediately execute the next step in the sequence.
    ///
    /// Generally, periodic execution of the [Self::run] method should be performed while
    /// [Self::state] is not [SequenceExecutionHelperState::Idle].
    ///
    /// # Arguments
    ///
    /// * `table` - This table contains the sequence tables to reach the mode previously loaded
    ///   with [Self::load]
    /// * `sender` - The sender to send mode requests to the components
    /// * `children_mode_store` - The mode store vector to keep track of the mode states of
    ///    children components
    pub fn run(
        &mut self,
        table: &SequenceModeTables,
        sender: &impl ModeRequestSender,
        children_mode_store: &mut ModeStoreVec,
    ) -> Result<ModeCommandingResult, GenericTargetedMessagingError> {
        if self.state == SequenceExecutionHelperState::Idle {
            return Ok(ModeCommandingResult::CommandingDone);
        }
        if self.state == SequenceExecutionHelperState::AwaitingCheckSuccess {
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
                    children_mode_store,
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
                        children_mode_store,
                    )
                }
            }
        }
    }

    /// Retrieve the currently loaded target mode
    pub fn target_mode(&self) -> Option<Mode> {
        self.target_mode
    }

    /// Confirm that a sequence which is awaiting a success check is done
    pub fn confirm_sequence_done(&mut self) {
        if let SequenceExecutionHelperState::AwaitingCheckSuccess = self.state {
            self.state = SequenceExecutionHelperState::Idle;
        }
    }

    /// Internal state of the execution helper.
    pub fn state(&self) -> SequenceExecutionHelperState {
        self.state
    }

    pub fn awaiting_check_success(&self) -> bool {
        self.state == SequenceExecutionHelperState::AwaitingCheckSuccess
    }

    pub fn current_sequence_index(&self) -> Option<usize> {
        self.current_sequence_index
    }

    /// Execute a sequence at the given sequence index for a given [SequenceTablesMapValue].
    ///
    /// This method calls [Self::execute_sequence] and maps the result to a [ModeCommandingResult].
    /// It is also called by the [Self::run] method of this helper.
    pub fn execute_sequence_and_map_to_result(
        &mut self,
        seq_table_value: &SequenceTablesMapValue,
        sequence_idx: usize,
        sender: &impl ModeRequestSender,
        mode_store_vec: &mut ModeStoreVec,
    ) -> Result<ModeCommandingResult, GenericTargetedMessagingError> {
        if self.state() == SequenceExecutionHelperState::Idle || self.request_id.is_none() {
            return Ok(ModeCommandingResult::CommandingDone);
        }
        if Self::execute_sequence(
            self.request_id.unwrap(),
            &seq_table_value.entries[sequence_idx],
            sender,
            mode_store_vec,
        )? {
            self.state = SequenceExecutionHelperState::AwaitingCheckSuccess;
            Ok(ModeCommandingResult::AwaitingSuccessCheck)
        } else if seq_table_value.entries.len() - 1 == sequence_idx {
            self.state = SequenceExecutionHelperState::Idle;
            return Ok(ModeCommandingResult::CommandingDone);
        } else {
            self.current_sequence_index = Some(sequence_idx + 1);
            return Ok(ModeCommandingResult::CommandingStepDone);
        }
    }

    /// Generic stateless execution helper method.
    ///
    /// The [RequestId] and the [SequenceTableMapTable] to be executed are passed explicitely
    /// here. This method is called by [Self::execute_sequence_and_map_to_result].
    ///
    /// This method itereates through the entries of the given sequence table and sends out
    /// [ModeRequest]s to set the modes of the children according to the table entries.
    /// It also sets the reply awaition field in the children mode store where a success
    /// check is required to true.
    ///
    /// It returns whether any commanding success check is required by any entry in the table.
    pub fn execute_sequence(
        request_id: RequestId,
        map_table: &SequenceTableMapTable,
        sender: &impl ModeRequestSender,
        children_mode_store: &mut ModeStoreVec,
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
            if entry.check_success {
                children_mode_store.0.iter_mut().for_each(|val| {
                    if val.id() == entry.common.target_id {
                        val.awaiting_reply = true;
                    }
                });
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
    /// The helper is currently trying to keep a target mode.
    TargetKeeping,
    /// The helper is currently busy to command a mode.
    ModeCommanding,
}

#[derive(Debug, Default)]
pub enum SubsystemHelperResult {
    #[default]
    Idle,
    /// Result of a target keeping operation
    TargetKeeping(TargetKeepingResult),
    /// Result of a mode commanding operation
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

/// This is a helper object which can be used by a subsystem component to execute mode sequences
/// and perform target keeping.
///
/// This helper object tries to compose as much data and state information as possible which is
/// required for this process.
pub struct SubsystemCommandingHelper {
    /// Current mode of the owner subsystem.
    pub current_mode: ModeAndSubmode,
    /// State of the helper.
    pub state: ModeTreeHelperState,
    /// This data structure is used to track all mode children.
    pub children_mode_store: ModeStoreVec,
    /// This field is set when a mode sequence is executed. It is used to determine whether mode
    /// replies are relevant for reply awaition logic.
    pub active_request_id: Option<RequestId>,
    /// The primary data structure to keep the target state information for subsystem
    /// [modes][Mode]. it specifies the mode each child should have for a certain subsystem mode
    /// and is relevant for target keeping.
    pub target_tables: TargetModeTables,
    /// The primary data structure to keep the sequence commanding information for commanded
    /// subsystem [modes][Mode]. It specifies the actual commands and the order they should be
    /// sent in to reach a certain [mode][Mode].
    pub sequence_tables: SequenceModeTables,
    /// The sequence execution helper is used to execute sequences in the [Self::sequence_tables].
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
    /// Create a new substem commanding helper with an intial [ModeTreeHelperState::Idle] state,
    /// an empty mode children store and empty target and sequence mode tables.
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

    /// Add a mode child to the internal [Self::children_mode_store].
    pub fn add_mode_child(&mut self, child: ComponentId, mode: ModeAndSubmode) {
        self.children_mode_store.add_component(child, mode);
    }

    /// Add a target mode table and an associated sequence mode table.
    pub fn add_target_and_sequence_table(
        &mut self,
        mode: Mode,
        target_table_val: TargetTablesMapValue,
        sequence_table_val: SequenceTablesMapValue,
    ) {
        self.target_tables.0.insert(mode, target_table_val);
        self.sequence_tables.0.insert(mode, sequence_table_val);
    }

    /// Starts a command sequence for a given [mode][Mode].
    pub fn start_command_sequence(
        &mut self,
        mode: Mode,
        request_id: RequestId,
    ) -> Result<(), ModeDoesNotExistError> {
        self.seq_exec_helper
            .load(mode, request_id, &self.sequence_tables)?;
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
                    self.current_mode =
                        ModeAndSubmode::new(self.seq_exec_helper.target_mode().unwrap(), 0);
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
                let still_awating_replies = self.children_mode_store.mode_reply_handler(
                    sender_id,
                    mode_and_submode,
                    handle_awaition,
                );
                if self.state == ModeTreeHelperState::ModeCommanding
                    && !still_awating_replies.unwrap_or(false)
                {
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
        let val_mut = self
            .children_mode_store
            .get_mut(child)
            .ok_or(TargetNotInModeStoreError(child))?;
        val_mut.mode_and_submode = mode;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;
    use std::collections::VecDeque;

    use crate::{
        mode::{ModeAndSubmode, ModeRequest, ModeRequestSender, UNKNOWN_MODE},
        mode_tree::{
            ModeStoreProvider, ModeStoreVec, SequenceModeTables, SequenceTableEntry,
            SequenceTableMapTable, SequenceTablesMapValue,
        },
        queue::GenericTargetedMessagingError,
        request::RequestId,
        subsystem::SequenceExecutionHelperState,
        ComponentId,
    };

    use super::SequenceExecutionHelper;

    #[derive(Debug)]
    pub enum ExampleTargetId {
        Target0 = 1,
        Target1 = 2,
        Target2 = 3,
    }

    #[derive(Debug)]
    pub enum ExampleMode {
        Mode0 = 1,
        Mode1 = 2,
        Mode2 = 3,
    }

    #[derive(Default)]
    pub struct ModeReqSenderMock {
        pub requests: RefCell<VecDeque<(RequestId, ComponentId, ModeRequest)>>,
    }

    impl ModeRequestSender for ModeReqSenderMock {
        fn local_channel_id(&self) -> crate::ComponentId {
            0
        }

        fn send_mode_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            request: ModeRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.requests
                .borrow_mut()
                .push_back((request_id, target_id, request));
            Ok(())
        }
    }

    fn create_default_mode_store() -> ModeStoreVec {
        let mut mode_store = ModeStoreVec::default();
        mode_store.add_component(ExampleTargetId::Target0 as u64, UNKNOWN_MODE);
        mode_store.add_component(ExampleTargetId::Target1 as u64, UNKNOWN_MODE);
        mode_store.add_component(ExampleTargetId::Target2 as u64, UNKNOWN_MODE);
        mode_store
    }

    fn create_simple_sample_seq_table() -> SequenceModeTables {
        let mut table = SequenceModeTables::default();
        let mut table_val = SequenceTablesMapValue::new("MODE_0");
        let mut table_seq_0 = SequenceTableMapTable::new("MODE_0_SEQ_0");
        table_seq_0.add_entry(SequenceTableEntry::new(
            "TARGET_0",
            ExampleTargetId::Target0 as u64,
            ModeAndSubmode::new(ExampleMode::Mode0 as u32, 0),
            false,
        ));
        table_seq_0.add_entry(SequenceTableEntry::new(
            "TARGET_1",
            ExampleTargetId::Target1 as u64,
            ModeAndSubmode::new(ExampleMode::Mode1 as u32, 0),
            true,
        ));
        table_val.add_sequence_table(table_seq_0);
        table.0.insert(ExampleMode::Mode0 as u32, table_val);
        table
    }

    #[test]
    fn test_sequence_execution_helper() {
        let sender = ModeReqSenderMock::default();
        let mut mode_store = create_default_mode_store();
        let mut execution_helper = SequenceExecutionHelper::new();
        assert_eq!(execution_helper.state(), SequenceExecutionHelperState::Idle);
        let simple_table = create_simple_sample_seq_table();
        execution_helper
            .load(ExampleMode::Mode0 as u32, 1, &simple_table)
            .unwrap();
        assert_eq!(execution_helper.state(), SequenceExecutionHelperState::Busy);
        execution_helper
            .run(&simple_table, &sender, &mut mode_store)
            .expect("sequence exeecution helper run failure");
        // TODO: continue tests
    }

    // TODO: Test subsystem commanding helper
}
