use crate::{
    ComponentId,
    health::{HealthState, HealthTableProvider},
    mode::{Mode, ModeAndSubmode, ModeReply, ModeRequest, ModeRequestSender, UNKNOWN_MODE_VAL},
    mode_tree::{
        ModeStoreProvider, ModeStoreVec, SequenceModeTables, SequenceTableMapTable,
        SequenceTablesMapValue, TargetModeTables, TargetNotInModeStoreError, TargetTablesMapValue,
    },
    queue::GenericTargetedMessagingError,
    request::{GenericMessage, RequestId},
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
    AwaitingSuccessCheck,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ModeCommandingResult {
    /// The commanding of all children is finished
    Done,
    /// One step of a commanding chain is finished
    StepDone,
    /// Reply awaition is required for some children
    AwaitingSuccessCheck,
}

#[derive(Debug, thiserror::Error)]
#[error("mode {0} does not exist")]
pub struct ModeDoesNotExistError(Mode);

#[derive(Debug, thiserror::Error)]
pub enum StartSequenceError {
    #[error("mode {0} does not exist")]
    ModeDoesNotExist(#[from] ModeDoesNotExistError),
    #[error("invalid request ID")]
    InvalidRequestId(RequestId),
}

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
    last_sequence_index: Option<usize>,
}

impl Default for SequenceExecutionHelper {
    fn default() -> Self {
        Self {
            target_mode: None,
            state: SequenceExecutionHelperState::Idle,
            request_id: None,
            current_sequence_index: None,
            last_sequence_index: None,
        }
    }
}

pub trait IsChildCommandable {
    fn is_commandable(&self, id: ComponentId) -> bool;
}

impl<T> IsChildCommandable for T
where
    T: HealthTableProvider,
{
    fn is_commandable(&self, id: ComponentId) -> bool {
        self.health(id)
            .is_none_or(|h| h != HealthState::ExternalControl)
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
    /// * [ModeCommandingResult::Done] - The sequence is done. The user can load a new
    ///   sequence now without overwriting the last one. The sequence executor is in
    ///   [SequenceExecutionHelperState::Idle] again.
    /// * [ModeCommandingResult::StepDone] - The sequence has advanced one step. The user
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
    ///   children components
    pub fn run(
        &mut self,
        table: &SequenceModeTables,
        sender: &impl ModeRequestSender,
        children_mode_store: &mut ModeStoreVec,
        is_commandable: &impl IsChildCommandable,
    ) -> Result<ModeCommandingResult, GenericTargetedMessagingError> {
        if self.state == SequenceExecutionHelperState::Idle {
            return Ok(ModeCommandingResult::Done);
        }
        if self.state == SequenceExecutionHelperState::AwaitingSuccessCheck {
            return Ok(ModeCommandingResult::AwaitingSuccessCheck);
        }
        if self.target_mode.is_none() {
            return Ok(ModeCommandingResult::Done);
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
                    is_commandable,
                )
            }
            None => {
                // Find the first sequence
                let seq_table_value = table.0.get(&self.target_mode.unwrap()).unwrap();
                self.last_sequence_index = Some(seq_table_value.entries.len() - 1);
                if seq_table_value.entries.is_empty() {
                    Ok(ModeCommandingResult::Done)
                } else {
                    self.current_sequence_index = Some(0);
                    self.execute_sequence_and_map_to_result(
                        seq_table_value,
                        0,
                        sender,
                        children_mode_store,
                        is_commandable,
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
        if let SequenceExecutionHelperState::AwaitingSuccessCheck = self.state {
            self.state = SequenceExecutionHelperState::Busy;
            if let (Some(last_sequence_index), Some(current_sequence_index)) =
                (self.last_sequence_index, self.current_sequence_index)
            {
                if current_sequence_index == last_sequence_index {
                    self.state = SequenceExecutionHelperState::Idle;
                }
            }
            self.current_sequence_index = Some(self.current_sequence_index.unwrap() + 1);
        }
    }

    /// Internal state of the execution helper.
    pub fn state(&self) -> SequenceExecutionHelperState {
        self.state
    }

    pub fn request_id(&self) -> Option<RequestId> {
        self.request_id
    }

    pub fn set_request_id(&mut self, request_id: RequestId) {
        self.request_id = Some(request_id);
    }

    pub fn awaiting_success_check(&self) -> bool {
        self.state == SequenceExecutionHelperState::AwaitingSuccessCheck
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
        is_commandable: &impl IsChildCommandable,
    ) -> Result<ModeCommandingResult, GenericTargetedMessagingError> {
        if self.state() == SequenceExecutionHelperState::Idle || self.request_id.is_none() {
            return Ok(ModeCommandingResult::Done);
        }
        if Self::execute_sequence(
            self.request_id.unwrap(),
            &seq_table_value.entries[sequence_idx],
            sender,
            mode_store_vec,
            is_commandable,
        )? {
            self.state = SequenceExecutionHelperState::AwaitingSuccessCheck;
            Ok(ModeCommandingResult::AwaitingSuccessCheck)
        } else if seq_table_value.entries.len() - 1 == sequence_idx {
            self.state = SequenceExecutionHelperState::Idle;
            Ok(ModeCommandingResult::Done)
        } else {
            self.current_sequence_index = Some(sequence_idx + 1);
            Ok(ModeCommandingResult::StepDone)
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
        commandable: &impl IsChildCommandable,
    ) -> Result<bool, GenericTargetedMessagingError> {
        let mut some_succes_check_required = false;
        for entry in &map_table.entries {
            if !commandable.is_commandable(entry.common.target_id) {
                continue;
            }
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

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum ModeTreeHelperState {
    #[default]
    Idle,
    /// The helper is currently trying to keep a target mode.
    TargetKeeping,
    /// The helper is currently busy to command a mode.
    ModeCommanding,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum SubsystemHelperResult {
    #[default]
    Idle,
    /// Busy with target keeping.
    TargetKeeping,
    /// Result of a mode commanding operation
    ModeCommanding(ModeCommandingResult),
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
    /// Mode command has failed, for example while executing a mode table.
    #[error("mode command failed")]
    ModeCommmandFailure {
        /// Table index of the sequence table entry which failed.
        seq_table_index: Option<usize>,
    },
    /// Target mode keeping violation.
    #[error("target keeping violation")]
    TargetKeepingViolation {
        /// Table index of the sequence table entry which failed.
        fallback_mode: Option<Mode>,
    },
}

/// This is a helper object which can be used by a subsystem component to execute mode sequences
/// and perform target keeping.
///
/// This helper object tries to compose as much data and state information as possible which is
/// required for this process.
pub struct SubsystemCommandingHelper {
    /// State of the helper.
    state: ModeTreeHelperState,
    /// Current mode of the owner subsystem.
    current_mode: Mode,
    /// This data structure is used to track all mode children.
    pub children_mode_store: ModeStoreVec,
    /// This field is set when a mode sequence is executed. It is used to determine whether mode
    /// replies are relevant for reply awaition logic.
    active_internal_request_id: Option<RequestId>,
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
            current_mode: UNKNOWN_MODE_VAL,
            state: Default::default(),
            children_mode_store: Default::default(),
            active_internal_request_id: None,
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
            current_mode: UNKNOWN_MODE_VAL,
            state: ModeTreeHelperState::Idle,
            children_mode_store,
            active_internal_request_id: None,
            target_tables,
            sequence_tables,
            seq_exec_helper: Default::default(),
        }
    }

    pub fn state(&self) -> ModeTreeHelperState {
        self.state
    }

    pub fn mode(&self) -> Mode {
        self.current_mode
    }

    pub fn request_id(&self) -> Option<RequestId> {
        self.active_internal_request_id.map(|v| v >> 8)
    }

    /// This returns the internal request ID, which is the regular [Self::request_id] specified
    /// by the user shifter 8 to the right and then increment with the current sequence commanding
    /// step. The value can still be retrieved because it might be required for reply verification.
    ///
    /// The state machine specifies this request ID for all mode commands related to the
    /// current step of sequence commanding.
    pub fn internal_request_id(&self) -> Option<RequestId> {
        self.active_internal_request_id
    }

    /// Retrieve the fallback mode for the current mode of the subsystem by trying to retrieve
    /// it from the target table.
    ///
    /// If the current mode does not have a fallback mode, returns [None].
    /// If the current mode is not inside the target table, returns a [ModeDoesNotExistError].
    /// The fallback mode can and should be commanded when a target keeping violation was detected
    /// or after self-commanding to the current mode has failed, which can happen after a failed
    /// mode table execution.
    pub fn fallback_mode(&self) -> Result<Option<Mode>, ModeDoesNotExistError> {
        self.target_tables
            .0
            .get(&self.current_mode)
            .ok_or(ModeDoesNotExistError(self.current_mode))
            .map(|v| v.fallback_mode)
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
    ///
    /// # Arguments
    ///
    /// - `mode` - The mode to command
    /// - `request_id` - Request ID associated with the command sequence. The value of this value
    ///   should not be larger than the maximum possible value for 24 bits: (2 ^ 24) - 1 = 16777215
    ///   because 8 bits are reserved for internal sequence index tracking.
    pub fn start_command_sequence(
        &mut self,
        mode: Mode,
        request_id: RequestId,
    ) -> Result<(), StartSequenceError> {
        if request_id > 2_u32.pow(24) - 1 {
            return Err(StartSequenceError::InvalidRequestId(request_id));
        }
        self.active_internal_request_id = Some(request_id << 8);
        self.seq_exec_helper.load(
            mode,
            self.active_internal_request_id.unwrap(),
            &self.sequence_tables,
        )?;
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
        is_commandable: &impl IsChildCommandable,
    ) -> Result<SubsystemHelperResult, ModeTreeHelperError> {
        if let Some(reply) = opt_reply {
            if self.handle_mode_reply(&reply)? {
                if self.seq_exec_helper.state() == SequenceExecutionHelperState::Idle {
                    self.transition_to_target_keeping();
                    return Ok(SubsystemHelperResult::ModeCommanding(
                        ModeCommandingResult::Done,
                    ));
                }
                return Ok(SubsystemHelperResult::ModeCommanding(
                    ModeCommandingResult::StepDone,
                ));
            }
        }
        match self.state {
            ModeTreeHelperState::Idle => Ok(SubsystemHelperResult::Idle),
            ModeTreeHelperState::TargetKeeping => {
                // We check whether the current mode is modelled by a target table first.
                if let Some(target_table) = self.target_tables.0.get(&self.current_mode) {
                    self.perform_target_keeping(target_table)?;
                }
                Ok(SubsystemHelperResult::TargetKeeping)
            }
            ModeTreeHelperState::ModeCommanding => {
                let result = self.seq_exec_helper.run(
                    &self.sequence_tables,
                    req_sender,
                    &mut self.children_mode_store,
                    is_commandable,
                )?;
                match result {
                    ModeCommandingResult::Done => {
                        // By default, the helper will automatically transition into the target keeping
                        // mode after an executed sequence.
                        self.transition_to_target_keeping();
                    }
                    ModeCommandingResult::StepDone => {
                        // Normally, this step is done after all replies were received, but if no
                        // reply checking is required for a command sequence, the step would never
                        // be performed, so this function needs to be called here as well.
                        self.update_internal_req_id();
                    }
                    ModeCommandingResult::AwaitingSuccessCheck => (),
                }
                Ok(result.into())
            }
        }
    }

    fn transition_to_target_keeping(&mut self) {
        self.state = ModeTreeHelperState::TargetKeeping;
        self.current_mode = self.seq_exec_helper.target_mode().unwrap();
    }

    fn perform_target_keeping(
        &self,
        target_table: &TargetTablesMapValue,
    ) -> Result<(), ModeTreeHelperError> {
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
                return Err(ModeTreeHelperError::TargetKeepingViolation {
                    fallback_mode: target_table.fallback_mode,
                });
            }
        }
        Ok(())
    }

    fn update_internal_req_id(&mut self) {
        let new_internal_req_id = (self.request_id().unwrap() << 8)
            | self.seq_exec_helper.current_sequence_index().unwrap() as u32;
        self.seq_exec_helper.set_request_id(new_internal_req_id);
        self.active_internal_request_id = Some(new_internal_req_id);
    }

    // Handles a mode reply message and returns whether the reply completes a step of sequence
    // commanding.
    fn handle_mode_reply(
        &mut self,
        reply: &GenericMessage<ModeReply>,
    ) -> Result<bool, ModeTreeHelperError> {
        if !self.children_mode_store.has_component(reply.sender_id()) {
            return Ok(false);
        }
        let mut generic_mode_reply_handler =
            |sender_id, mode_and_submode: Option<ModeAndSubmode>, success: bool| {
                let mut partial_step_done = false;
                // Tying the reply awaition to the request ID ensures that something like replies
                // belonging to older requests do not interfere with the completion handling of
                // the mode commanding. This is important for forced mode commands.
                let mut handle_awaition = false;
                if self.state == ModeTreeHelperState::ModeCommanding
                    && self.active_internal_request_id.is_some()
                    && reply.request_id() == self.active_internal_request_id.unwrap()
                {
                    handle_awaition = true;
                }
                let still_awating_replies = self.children_mode_store.mode_reply_handler(
                    sender_id,
                    mode_and_submode,
                    handle_awaition,
                );
                if self.state == ModeTreeHelperState::ModeCommanding
                    && handle_awaition
                    && !still_awating_replies.unwrap_or(false)
                {
                    self.seq_exec_helper.confirm_sequence_done();
                    self.update_internal_req_id();
                    partial_step_done = true;
                }
                if !success && self.state == ModeTreeHelperState::ModeCommanding {
                    // The user has to decide how to proceed.
                    self.state = ModeTreeHelperState::Idle;
                    return Err(ModeTreeHelperError::ModeCommmandFailure {
                        seq_table_index: self.seq_exec_helper.current_sequence_index(),
                    });
                }
                Ok(partial_step_done)
            };
        match reply.message {
            ModeReply::ModeInfo(mode_and_submode) => {
                generic_mode_reply_handler(reply.sender_id(), Some(mode_and_submode), true)
            }
            ModeReply::ModeReply(mode_and_submode) => {
                generic_mode_reply_handler(reply.sender_id(), Some(mode_and_submode), true)
            }
            ModeReply::CantReachMode(_) => {
                generic_mode_reply_handler(reply.sender_id(), None, false)
            }
            ModeReply::WrongMode { reached, .. } => {
                generic_mode_reply_handler(reply.sender_id(), Some(reached), true)
            }
        }
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
    use super::*;

    use crate::{
        ComponentId,
        mode::{
            Mode, ModeAndSubmode, ModeReply, ModeRequest, UNKNOWN_MODE,
            tests::{ModeReqSenderMock, ModeReqWrapper},
        },
        mode_tree::{
            ModeStoreProvider, ModeStoreVec, SequenceModeTables, SequenceTableEntry,
            SequenceTableMapTable, SequenceTablesMapValue, TargetModeTables,
        },
        queue::GenericTargetedMessagingError,
        request::{GenericMessage, MessageMetadata, RequestId},
        subsystem::{ModeCommandingResult, ModeTreeHelperState, SequenceExecutionHelperState},
    };

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

    #[derive(Debug, Default)]
    pub struct IsCommandableMock {
        pub commandable_map: std::collections::HashMap<ComponentId, bool>,
    }

    impl IsChildCommandable for IsCommandableMock {
        fn is_commandable(&self, id: ComponentId) -> bool {
            self.commandable_map.get(&id).copied().unwrap_or(true)
        }
    }

    pub struct SequenceExecutorTestbench {
        pub sender: ModeReqSenderMock,
        pub mode_store: ModeStoreVec,
        pub seq_tables: SequenceModeTables,
        pub execution_helper: SequenceExecutionHelper,
        pub is_commandable_mock: IsCommandableMock,
    }

    impl SequenceExecutorTestbench {
        pub fn new() -> Self {
            let mode_store = create_default_mode_store();
            let (seq_tables, _) = create_simple_sample_seq_tables();
            Self {
                sender: ModeReqSenderMock::default(),
                mode_store,
                seq_tables,
                execution_helper: SequenceExecutionHelper::new(),
                is_commandable_mock: IsCommandableMock::default(),
            }
        }

        pub fn get_mode_table(&mut self, mode: ExampleMode) -> &mut SequenceTablesMapValue {
            self.seq_tables.0.get_mut(&(mode as Mode)).unwrap()
        }

        pub fn run(&mut self) -> Result<ModeCommandingResult, GenericTargetedMessagingError> {
            self.execution_helper.run(
                &self.seq_tables,
                &self.sender,
                &mut self.mode_store,
                &self.is_commandable_mock,
            )
        }

        fn check_run_is_no_op(&mut self) {
            // Assure that no unexpected behaviour occurs.
            assert_eq!(
                self.execution_helper
                    .run(
                        &self.seq_tables,
                        &self.sender,
                        &mut self.mode_store,
                        &self.is_commandable_mock
                    )
                    .unwrap(),
                ModeCommandingResult::Done
            );
            assert_eq!(
                self.execution_helper.state(),
                SequenceExecutionHelperState::Idle
            );
            assert!(self.sender.requests.borrow().is_empty());
        }

        fn generic_checks_subsystem_md1_step0(&mut self, expected_req_id: RequestId) {
            assert_eq!(
                self.execution_helper.target_mode().unwrap(),
                ExampleMode::Mode1 as Mode
            );
            assert_eq!(self.sender.requests.borrow().len(), 2);
            let req_0 = self.sender.requests.get_mut().pop_front().unwrap();
            assert_eq!(req_0.target_id, ExampleTargetId::Target0 as ComponentId);
            assert_eq!(req_0.request_id, expected_req_id);
            assert_eq!(
                req_0.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD1_ST0_TGT0_MODE,
                    forced: false
                }
            );
            let req_1 = self.sender.requests.borrow_mut().pop_front().unwrap();
            assert_eq!(req_1.target_id, ExampleTargetId::Target1 as ComponentId);
            assert_eq!(
                req_1.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD1_ST0_TGT1_MODE,
                    forced: false
                }
            );
        }
        fn generic_checks_subsystem_md1_step1(&mut self, expected_req_id: RequestId) {
            assert_eq!(
                self.execution_helper.target_mode().unwrap(),
                ExampleMode::Mode1 as Mode
            );
            assert_eq!(self.sender.requests.borrow().len(), 1);
            let req_0 = self.sender.requests.get_mut().pop_front().unwrap();
            assert_eq!(req_0.target_id, ExampleTargetId::Target2 as ComponentId);
            assert_eq!(req_0.request_id, expected_req_id);
            assert_eq!(
                req_0.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD1_ST1_TGT2_MODE,
                    forced: false
                }
            );
        }

        fn generic_checks_subsystem_md0(&mut self, expected_req_id: RequestId) {
            assert_eq!(
                self.execution_helper.target_mode().unwrap(),
                ExampleMode::Mode0 as Mode
            );
            assert_eq!(self.execution_helper.current_sequence_index().unwrap(), 0);
            assert_eq!(self.sender.requests.borrow().len(), 2);
            let req_0 = self.sender.requests.get_mut().pop_front().unwrap();
            assert_eq!(req_0.target_id, ExampleTargetId::Target0 as ComponentId);
            assert_eq!(req_0.request_id, expected_req_id);
            assert_eq!(
                req_0.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD0_TGT0_MODE,
                    forced: false
                }
            );
            let req_1 = self.sender.requests.borrow_mut().pop_front().unwrap();
            assert_eq!(req_1.target_id, ExampleTargetId::Target1 as ComponentId);
            assert_eq!(
                req_1.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD0_TGT1_MODE,
                    forced: false
                }
            );
        }
    }

    fn create_default_mode_store() -> ModeStoreVec {
        let mut mode_store = ModeStoreVec::default();
        mode_store.add_component(ExampleTargetId::Target0 as ComponentId, UNKNOWN_MODE);
        mode_store.add_component(ExampleTargetId::Target1 as ComponentId, UNKNOWN_MODE);
        mode_store.add_component(ExampleTargetId::Target2 as ComponentId, UNKNOWN_MODE);
        mode_store
    }

    fn create_simple_sample_seq_tables() -> (SequenceModeTables, TargetModeTables) {
        let mut seq_tables = SequenceModeTables::default();
        // Mode 0 - One step command
        let mut table_val = SequenceTablesMapValue::new("MODE_0");
        let mut table_seq_0 = SequenceTableMapTable::new("MODE_0_SEQ_0");
        table_seq_0.add_entry(SequenceTableEntry::new(
            "TARGET_0",
            ExampleTargetId::Target0 as ComponentId,
            SUBSYSTEM_MD0_TGT0_MODE,
            false,
        ));
        table_seq_0.add_entry(SequenceTableEntry::new(
            "TARGET_1",
            ExampleTargetId::Target1 as ComponentId,
            SUBSYSTEM_MD0_TGT1_MODE,
            false,
        ));
        table_val.add_sequence_table(table_seq_0);
        seq_tables.0.insert(ExampleMode::Mode0 as u32, table_val);

        // Mode 1 - Multi Step command
        let mut table_val = SequenceTablesMapValue::new("MODE_1");
        let mut table_seq_0 = SequenceTableMapTable::new("MODE_1_SEQ_0");
        table_seq_0.add_entry(SequenceTableEntry::new(
            "MD1_SEQ0_TGT0",
            ExampleTargetId::Target0 as ComponentId,
            SUBSYSTEM_MD1_ST0_TGT0_MODE,
            false,
        ));
        table_seq_0.add_entry(SequenceTableEntry::new(
            "MD1_SEQ0_TGT1",
            ExampleTargetId::Target1 as ComponentId,
            SUBSYSTEM_MD1_ST0_TGT1_MODE,
            false,
        ));
        table_val.add_sequence_table(table_seq_0);
        let mut table_seq_1 = SequenceTableMapTable::new("MODE_1_SEQ_1");
        table_seq_1.add_entry(SequenceTableEntry::new(
            "MD1_SEQ1_TGT2",
            ExampleTargetId::Target2 as ComponentId,
            SUBSYSTEM_MD1_ST1_TGT2_MODE,
            false,
        ));
        table_val.add_sequence_table(table_seq_1);
        seq_tables.0.insert(ExampleMode::Mode1 as u32, table_val);

        let mode_tables = TargetModeTables::default();
        // TODO: Write mode tables.
        (seq_tables, mode_tables)
    }

    pub struct SubsystemHelperTestbench {
        pub sender: ModeReqSenderMock,
        pub helper: SubsystemCommandingHelper,
        pub is_commandable_mock: IsCommandableMock,
    }

    impl SubsystemHelperTestbench {
        pub fn new() -> Self {
            let (sequence_tables, target_tables) = create_simple_sample_seq_tables();
            Self {
                sender: ModeReqSenderMock::default(),
                helper: SubsystemCommandingHelper::new(
                    create_default_mode_store(),
                    target_tables,
                    sequence_tables,
                ),
                is_commandable_mock: IsCommandableMock::default(),
            }
        }

        pub fn start_command_sequence(
            &mut self,
            mode: ExampleMode,
            request_id: RequestId,
        ) -> Result<(), StartSequenceError> {
            self.helper.start_command_sequence(mode as Mode, request_id)
        }

        pub fn send_announce_mode_cmd_to_children(
            &mut self,
            request_id: RequestId,
            recursive: bool,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.helper
                .send_announce_mode_cmd_to_children(request_id, &self.sender, recursive)
        }

        pub fn get_sequence_tables(&mut self, mode: ExampleMode) -> &mut SequenceTablesMapValue {
            self.helper
                .sequence_tables
                .0
                .get_mut(&(mode as Mode))
                .unwrap()
        }

        pub fn state_machine(
            &mut self,
            opt_reply: Option<GenericMessage<ModeReply>>,
        ) -> Result<SubsystemHelperResult, ModeTreeHelperError> {
            self.helper
                .state_machine(opt_reply, &self.sender, &self.is_commandable_mock)
        }

        pub fn generic_checks_subsystem_md0(&mut self, expected_req_id: RequestId) {
            assert_eq!(self.sender.requests.borrow().len(), 2);
            let req0 = self.sender.requests.borrow_mut().pop_front().unwrap();
            assert_eq!(req0.request_id, expected_req_id);
            assert_eq!(req0.target_id, ExampleTargetId::Target0 as ComponentId);
            assert_eq!(
                req0.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD0_TGT0_MODE,
                    forced: false
                }
            );

            let req1 = self.sender.requests.borrow_mut().pop_front().unwrap();
            assert_eq!(req1.request_id, expected_req_id);
            assert_eq!(req1.target_id, ExampleTargetId::Target1 as ComponentId);
            assert_eq!(
                req1.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD0_TGT1_MODE,
                    forced: false
                }
            );
        }

        pub fn generic_checks_subsystem_md1_step0(&mut self, expected_req_id: RequestId) {
            assert_eq!(self.sender.requests.borrow().len(), 2);
            let req0 = self.sender.requests.borrow_mut().pop_front().unwrap();
            assert_eq!(req0.request_id, expected_req_id);
            assert_eq!(req0.target_id, ExampleTargetId::Target0 as ComponentId);
            assert_eq!(
                req0.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD1_ST0_TGT0_MODE,
                    forced: false
                }
            );

            let req1 = self.sender.requests.borrow_mut().pop_front().unwrap();
            assert_eq!(req1.request_id, expected_req_id);
            assert_eq!(req1.target_id, ExampleTargetId::Target1 as ComponentId);
            assert_eq!(
                req1.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD1_ST0_TGT1_MODE,
                    forced: false
                }
            );
        }

        pub fn generic_checks_subsystem_md1_step1(&mut self, expected_req_id: RequestId) {
            assert_eq!(self.sender.requests.borrow().len(), 1);
            let req0 = self.sender.requests.borrow_mut().pop_front().unwrap();
            assert_eq!(req0.request_id, expected_req_id);
            assert_eq!(req0.target_id, ExampleTargetId::Target2 as ComponentId);
            assert_eq!(
                req0.request,
                ModeRequest::SetMode {
                    mode_and_submode: SUBSYSTEM_MD1_ST1_TGT2_MODE,
                    forced: false
                }
            );
        }
    }

    const SUBSYSTEM_MD0_TGT0_MODE: ModeAndSubmode =
        ModeAndSubmode::new(ExampleMode::Mode0 as u32, 0);
    const SUBSYSTEM_MD0_TGT1_MODE: ModeAndSubmode =
        ModeAndSubmode::new(ExampleMode::Mode1 as u32, 0);

    const SUBSYSTEM_MD1_ST0_TGT0_MODE: ModeAndSubmode =
        ModeAndSubmode::new(ExampleMode::Mode2 as u32, 0);
    const SUBSYSTEM_MD1_ST0_TGT1_MODE: ModeAndSubmode =
        ModeAndSubmode::new(ExampleMode::Mode0 as u32, 0);
    const SUBSYSTEM_MD1_ST1_TGT2_MODE: ModeAndSubmode =
        ModeAndSubmode::new(ExampleMode::Mode1 as u32, 0);

    #[test]
    fn test_init_state() {
        let execution_helper = SequenceExecutionHelper::new();
        assert_eq!(execution_helper.state(), SequenceExecutionHelperState::Idle);
        assert!(!execution_helper.awaiting_success_check());
        assert!(execution_helper.target_mode().is_none());
        assert!(execution_helper.current_sequence_index().is_none());
    }

    #[test]
    fn test_sequence_execution_helper_no_success_check() {
        let mut tb = SequenceExecutorTestbench::new();
        let expected_req_id = 1;
        tb.execution_helper
            .load(ExampleMode::Mode0 as u32, expected_req_id, &tb.seq_tables)
            .unwrap();
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Busy
        );
        assert!(!tb.execution_helper.awaiting_success_check());
        assert_eq!(
            tb.execution_helper.target_mode().unwrap(),
            ExampleMode::Mode0 as Mode
        );
        assert_eq!(
            tb.run().expect("sequence exeecution helper run failure"),
            ModeCommandingResult::Done
        );
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Idle
        );
        assert!(!tb.execution_helper.awaiting_success_check());
        tb.generic_checks_subsystem_md0(expected_req_id);
        tb.check_run_is_no_op();
    }

    #[test]
    fn test_sequence_execution_helper_with_success_check() {
        let mut tb = SequenceExecutorTestbench::new();
        let mode0_table = tb.get_mode_table(ExampleMode::Mode0);
        mode0_table.entries[0].entries[0].check_success = true;
        mode0_table.entries[0].entries[1].check_success = true;
        let expected_req_id = 1;
        tb.execution_helper
            .load(ExampleMode::Mode0 as u32, expected_req_id, &tb.seq_tables)
            .unwrap();

        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Busy
        );
        assert!(!tb.execution_helper.awaiting_success_check());
        assert_eq!(
            tb.execution_helper.target_mode().unwrap(),
            ExampleMode::Mode0 as Mode
        );
        assert_eq!(
            tb.run().expect("sequence exeecution helper run failure"),
            ModeCommandingResult::AwaitingSuccessCheck
        );
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::AwaitingSuccessCheck
        );
        // These are not cleared, even if the execution helper is already IDLE. This is okay for
        // now.
        assert!(tb.execution_helper.awaiting_success_check());
        tb.generic_checks_subsystem_md0(expected_req_id);
        tb.execution_helper.confirm_sequence_done();
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Idle
        );

        tb.check_run_is_no_op();
    }

    #[test]
    fn test_sequence_execution_helper_with_partial_check() {
        let mut tb = SequenceExecutorTestbench::new();
        let mode0_table = tb.get_mode_table(ExampleMode::Mode0);
        mode0_table.entries[0].entries[0].check_success = true;
        let expected_req_id = 1;
        tb.execution_helper
            .load(ExampleMode::Mode0 as u32, expected_req_id, &tb.seq_tables)
            .unwrap();

        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Busy
        );
        assert!(!tb.execution_helper.awaiting_success_check());
        assert_eq!(
            tb.run().expect("sequence execution helper run failure"),
            ModeCommandingResult::AwaitingSuccessCheck
        );
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::AwaitingSuccessCheck
        );
        // These are not cleared, even if the execution helper is already IDLE. This is okay for
        // now.
        assert!(tb.execution_helper.awaiting_success_check());
        tb.generic_checks_subsystem_md0(expected_req_id);
        tb.execution_helper.confirm_sequence_done();
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Idle
        );
        tb.check_run_is_no_op();
    }

    #[test]
    fn test_sequence_execution_helper_multi_step_no_success_check() {
        let mut tb = SequenceExecutorTestbench::new();
        let expected_req_id = 1;
        tb.execution_helper
            .load(ExampleMode::Mode1 as u32, expected_req_id, &tb.seq_tables)
            .unwrap();
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Busy
        );
        assert!(!tb.execution_helper.awaiting_success_check());
        assert_eq!(
            tb.execution_helper.target_mode().unwrap(),
            ExampleMode::Mode1 as Mode
        );
        assert_eq!(
            tb.run().expect("sequence execution helper run failure"),
            ModeCommandingResult::StepDone
        );
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Busy
        );
        assert!(!tb.execution_helper.awaiting_success_check());
        tb.generic_checks_subsystem_md1_step0(expected_req_id);
        assert_eq!(tb.execution_helper.current_sequence_index().unwrap(), 1);

        assert_eq!(
            tb.run().expect("sequence execution helper run failure"),
            ModeCommandingResult::Done
        );
        tb.generic_checks_subsystem_md1_step1(expected_req_id);
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Idle
        );
        tb.check_run_is_no_op();
    }

    #[test]
    fn test_sequence_execution_helper_multi_step_full_success_check() {
        let mut tb = SequenceExecutorTestbench::new();
        let expected_req_id = 1;
        tb.execution_helper
            .load(ExampleMode::Mode1 as u32, expected_req_id, &tb.seq_tables)
            .unwrap();
        let mode1_table = tb.get_mode_table(ExampleMode::Mode1);
        mode1_table.entries[0].entries[0].check_success = true;
        mode1_table.entries[0].entries[1].check_success = true;
        mode1_table.entries[1].entries[0].check_success = true;

        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::Busy
        );
        assert!(!tb.execution_helper.awaiting_success_check());
        assert_eq!(
            tb.execution_helper.target_mode().unwrap(),
            ExampleMode::Mode1 as Mode
        );
        assert_eq!(
            tb.run().expect("sequence execution helper run failure"),
            ModeCommandingResult::AwaitingSuccessCheck
        );
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::AwaitingSuccessCheck
        );
        assert!(tb.execution_helper.awaiting_success_check());
        tb.generic_checks_subsystem_md1_step0(expected_req_id);
        assert_eq!(tb.execution_helper.current_sequence_index().unwrap(), 0);
        tb.execution_helper.confirm_sequence_done();

        assert_eq!(
            tb.run().expect("sequence execution helper run failure"),
            ModeCommandingResult::AwaitingSuccessCheck
        );
        assert_eq!(
            tb.execution_helper.state(),
            SequenceExecutionHelperState::AwaitingSuccessCheck
        );
        assert!(tb.execution_helper.awaiting_success_check());
        assert_eq!(tb.execution_helper.current_sequence_index().unwrap(), 1);
        tb.generic_checks_subsystem_md1_step1(expected_req_id);
        tb.execution_helper.confirm_sequence_done();
        tb.check_run_is_no_op();
    }

    // TODO: Test subsystem commanding helper
    #[test]
    fn test_subsystem_helper_basic_state() {
        let tb = SubsystemHelperTestbench::new();
        assert_eq!(tb.helper.state(), ModeTreeHelperState::Idle);
        assert!(tb.helper.active_internal_request_id.is_none());
        assert_eq!(tb.helper.mode(), UNKNOWN_MODE_VAL);
        assert!(tb.helper.request_id().is_none());
    }

    #[test]
    fn test_subsystem_helper_announce_recursive() {
        let mut tb = SubsystemHelperTestbench::new();
        let expected_req_id = 1;
        tb.send_announce_mode_cmd_to_children(expected_req_id, true)
            .unwrap();
        assert_eq!(tb.sender.requests.borrow().len(), 3);
        let check_req = |req: ModeReqWrapper, target_id: ComponentId| {
            assert_eq!(req.target_id, target_id);
            assert_eq!(req.request_id, expected_req_id);
            assert_eq!(req.request, ModeRequest::AnnounceModeRecursive);
        };
        let req0 = tb.sender.requests.borrow_mut().pop_front().unwrap();
        check_req(req0, ExampleTargetId::Target0 as ComponentId);
        let req1 = tb.sender.requests.borrow_mut().pop_front().unwrap();
        check_req(req1, ExampleTargetId::Target1 as ComponentId);
        let req2 = tb.sender.requests.borrow_mut().pop_front().unwrap();
        check_req(req2, ExampleTargetId::Target2 as ComponentId);
    }

    #[test]
    fn test_subsystem_helper_announce() {
        let mut tb = SubsystemHelperTestbench::new();
        let expected_req_id = 1;
        tb.send_announce_mode_cmd_to_children(expected_req_id, false)
            .unwrap();
        assert_eq!(tb.sender.requests.borrow().len(), 3);
        let check_req = |req: ModeReqWrapper, target_id: ComponentId| {
            assert_eq!(req.target_id, target_id);
            assert_eq!(req.request_id, expected_req_id);
            assert_eq!(req.request, ModeRequest::AnnounceMode);
        };
        let req0 = tb.sender.requests.borrow_mut().pop_front().unwrap();
        check_req(req0, ExampleTargetId::Target0 as ComponentId);
        let req1 = tb.sender.requests.borrow_mut().pop_front().unwrap();
        check_req(req1, ExampleTargetId::Target1 as ComponentId);
        let req2 = tb.sender.requests.borrow_mut().pop_front().unwrap();
        check_req(req2, ExampleTargetId::Target2 as ComponentId);
    }

    #[test]
    fn test_subsystem_helper_cmd_mode0_no_success_checks() {
        let mut tb = SubsystemHelperTestbench::new();
        let expected_req_id = 1;
        tb.start_command_sequence(ExampleMode::Mode0, expected_req_id)
            .unwrap();
        assert_eq!(tb.helper.request_id().unwrap(), 1);
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.sender.requests.borrow().len(), 0);
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::Done)
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::TargetKeeping);
        assert_eq!(tb.helper.mode(), ExampleMode::Mode0 as Mode);
        tb.generic_checks_subsystem_md0(tb.helper.internal_request_id().unwrap());
        // FSM call should be a no-op.
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::TargetKeeping
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::TargetKeeping);
        assert_eq!(tb.helper.mode(), ExampleMode::Mode0 as Mode);
    }

    #[test]
    fn test_subsystem_helper_cmd_mode1_no_success_checks() {
        let mut tb = SubsystemHelperTestbench::new();
        let expected_req_id = 1;
        tb.start_command_sequence(ExampleMode::Mode1, expected_req_id)
            .unwrap();
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.sender.requests.borrow().len(), 0);
        // Need to cache this before it is incremented, because it is incremented
        // immediately in the state machine (no reply checking)
        let expected_req_id = tb.helper.internal_request_id().unwrap();
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::StepDone)
        );
        // Assert that this was already incremented because no reply checking is necessary.
        assert_eq!(
            tb.helper.internal_request_id().unwrap(),
            expected_req_id + 1
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.helper.mode(), UNKNOWN_MODE_VAL);
        tb.generic_checks_subsystem_md1_step0(expected_req_id);
        // Second commanding step.
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::Done)
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::TargetKeeping);
        assert_eq!(tb.helper.mode(), ExampleMode::Mode1 as Mode);
        tb.generic_checks_subsystem_md1_step1(tb.helper.internal_request_id().unwrap());

        // FSM call should be a no-op.
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::TargetKeeping
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::TargetKeeping);
        assert_eq!(tb.helper.mode(), ExampleMode::Mode1 as Mode);
    }

    #[test]
    fn test_subsystem_helper_cmd_mode0_with_success_checks() {
        let mut tb = SubsystemHelperTestbench::new();
        let expected_req_id = 1;
        let seq_tables = tb.get_sequence_tables(ExampleMode::Mode0);
        seq_tables.entries[0].entries[0].check_success = true;
        seq_tables.entries[0].entries[1].check_success = true;
        tb.start_command_sequence(ExampleMode::Mode0, expected_req_id)
            .unwrap();
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.sender.requests.borrow().len(), 0);
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::AwaitingSuccessCheck)
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.helper.mode(), UNKNOWN_MODE_VAL);
        tb.generic_checks_subsystem_md0(tb.helper.internal_request_id().unwrap());
        let mode_reply_ok_0 = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target0 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD0_TGT0_MODE),
        );
        let mode_reply_ok_1 = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target1 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD0_TGT1_MODE),
        );
        // One success reply still expected.
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok_0)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::AwaitingSuccessCheck)
        );
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok_1)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::Done)
        );

        // FSM call should be a no-op.
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::TargetKeeping
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::TargetKeeping);
        assert_eq!(tb.helper.mode(), ExampleMode::Mode0 as Mode);
    }

    #[test]
    fn test_subsystem_helper_cmd_mode1_with_success_checks() {
        let mut tb = SubsystemHelperTestbench::new();
        let expected_req_id = 1;
        let seq_tables = tb.get_sequence_tables(ExampleMode::Mode1);
        seq_tables.entries[0].entries[0].check_success = true;
        seq_tables.entries[0].entries[1].check_success = true;
        seq_tables.entries[1].entries[0].check_success = true;
        tb.start_command_sequence(ExampleMode::Mode1, expected_req_id)
            .unwrap();
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.sender.requests.borrow().len(), 0);
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::AwaitingSuccessCheck)
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.helper.mode(), UNKNOWN_MODE_VAL);
        tb.generic_checks_subsystem_md1_step0(tb.helper.internal_request_id().unwrap());
        let mode_reply_ok_0 = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target0 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD0_TGT0_MODE),
        );
        let mode_reply_ok_1 = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target1 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD0_TGT1_MODE),
        );
        // One success reply still expected.
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok_0)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::AwaitingSuccessCheck)
        );
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok_1)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::StepDone)
        );

        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::AwaitingSuccessCheck)
        );
        let mode_reply_ok = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target2 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD1_ST1_TGT2_MODE),
        );
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::Done)
        );

        // FSM call should be a no-op.
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::TargetKeeping
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::TargetKeeping);
        assert_eq!(tb.helper.mode(), ExampleMode::Mode1 as Mode);
    }

    #[test]
    fn test_subsystem_helper_cmd_mode1_with_partial_success_checks_0() {
        let mut tb = SubsystemHelperTestbench::new();
        let expected_req_id = 1;
        let seq_tables = tb.get_sequence_tables(ExampleMode::Mode1);
        seq_tables.entries[0].entries[0].check_success = true;
        seq_tables.entries[0].entries[1].check_success = false;
        seq_tables.entries[1].entries[0].check_success = false;
        tb.start_command_sequence(ExampleMode::Mode1, expected_req_id)
            .unwrap();
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.sender.requests.borrow().len(), 0);
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::AwaitingSuccessCheck)
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.helper.mode(), UNKNOWN_MODE_VAL);
        tb.generic_checks_subsystem_md1_step0(tb.helper.internal_request_id().unwrap());
        let mode_reply_ok_0 = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target0 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD0_TGT0_MODE),
        );
        let mode_reply_ok_1 = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target1 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD0_TGT1_MODE),
        );
        // One success reply still expected.
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok_1)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::AwaitingSuccessCheck)
        );
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok_0)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::StepDone)
        );

        // Inserting the reply makes no difference: This call completes the sequence commanding.
        let mode_reply_ok = GenericMessage::new(
            MessageMetadata::new(expected_req_id, ExampleTargetId::Target2 as ComponentId),
            ModeReply::ModeInfo(SUBSYSTEM_MD1_ST1_TGT2_MODE),
        );
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::Done)
        );
        // The internal request ID is still cached.
        tb.generic_checks_subsystem_md1_step1(tb.helper.internal_request_id().unwrap());

        // FSM call should be a no-op.
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::TargetKeeping
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::TargetKeeping);
        assert_eq!(tb.helper.mode(), ExampleMode::Mode1 as Mode);
    }

    #[test]
    fn test_subsystem_helper_cmd_mode1_with_partial_success_checks_1() {
        let mut tb = SubsystemHelperTestbench::new();
        let expected_req_id = 1;
        let seq_tables = tb.get_sequence_tables(ExampleMode::Mode1);
        seq_tables.entries[0].entries[0].check_success = true;
        seq_tables.entries[0].entries[1].check_success = false;
        seq_tables.entries[1].entries[0].check_success = false;
        tb.start_command_sequence(ExampleMode::Mode1, expected_req_id)
            .unwrap();
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.sender.requests.borrow().len(), 0);
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::AwaitingSuccessCheck)
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::ModeCommanding);
        assert_eq!(tb.helper.mode(), UNKNOWN_MODE_VAL);
        tb.generic_checks_subsystem_md1_step0(tb.helper.internal_request_id().unwrap());
        let mode_reply_ok_0 = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target0 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD0_TGT0_MODE),
        );
        let mode_reply_ok_1 = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target1 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD0_TGT1_MODE),
        );
        // This completes the step, so the next FSM call will perform the next step
        // in sequence commanding.
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok_0)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::StepDone)
        );
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok_1)).unwrap(),
            SubsystemHelperResult::ModeCommanding(ModeCommandingResult::Done)
        );

        // Inserting the reply makes no difference: Sequence command is done and target keeping
        // is performed.
        let mode_reply_ok = GenericMessage::new(
            MessageMetadata::new(
                tb.helper.internal_request_id().unwrap(),
                ExampleTargetId::Target2 as ComponentId,
            ),
            ModeReply::ModeInfo(SUBSYSTEM_MD1_ST1_TGT2_MODE),
        );
        assert_eq!(
            tb.state_machine(Some(mode_reply_ok)).unwrap(),
            SubsystemHelperResult::TargetKeeping
        );
        // The internal request ID is still cached.
        tb.generic_checks_subsystem_md1_step1(tb.helper.internal_request_id().unwrap());

        // FSM call should be a no-op.
        assert_eq!(
            tb.state_machine(None).unwrap(),
            SubsystemHelperResult::TargetKeeping
        );
        assert_eq!(tb.helper.state(), ModeTreeHelperState::TargetKeeping);
        assert_eq!(tb.helper.mode(), ExampleMode::Mode1 as Mode);
    }
}
