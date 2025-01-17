use core::cell::Cell;
use satrs::mode::{
    Mode, ModeError, ModeProvider, ModeReplyReceiver, ModeReplySender, ModeRequestHandler,
    ModeRequestHandlerMpscBounded, ModeRequestReceiver, ModeRequestSender,
    ModeRequestorAndHandlerMpscBounded, ModeRequestorOneChildBoundedMpsc, INVALID_MODE,
    UNKNOWN_MODE,
};
use satrs::mode_tree::{
    connect_mode_nodes, ModeChild, ModeNode, ModeParent, ModeStoreProvider, SequenceTableEntry,
    SequenceTableMapTable, TargetNotInModeStoreError, TargetTableEntry,
};
use satrs::mode_tree::{
    ModeStoreVec, SequenceModeTables, SequenceTablesMapValue, TargetModeTables,
    TargetTablesMapValue,
};
use satrs::request::{MessageMetadata, RequestId};
use satrs::res_code::ResultU16;
use satrs::subsystem::{
    ModeCommandingResult, ModeDoesNotExistError, SequenceExecutionHelper, TargetKeepingResult,
};
use satrs::{
    mode::{ModeAndSubmode, ModeReply, ModeRequest},
    queue::GenericTargetedMessagingError,
    request::GenericMessage,
    ComponentId,
};
use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::{println, sync::mpsc};

pub enum DefaultMode {
    OFF = 0,
    ON = 1,
    NORMAL = 2,
}

#[derive(Debug)]
pub enum AcsMode {
    OFF = 0,
    SAFE = 1,
    IDLE = 2,
}

pub enum TestComponentId {
    MagnetometerDevice0 = 1,
    MagnetometerDevice1 = 2,
    MagnetorquerDevice = 5,
    ReactionWheelDevice = 6,
    StartrackerDevice = 7,
    MgtDevManager = 8,
    ReactionWheelAssembly = 10,
    MagnetometerAssembly = 11,
    AcsController = 14,
    AcsSubsystem = 15,
    PusModeService = 16,
}

pub type RequestSenderType = mpsc::SyncSender<GenericMessage<ModeRequest>>;
pub type ReplySenderType = mpsc::SyncSender<GenericMessage<ModeReply>>;

#[derive(Debug, Default, PartialEq, Eq)]
pub enum ModeTreeHelperState {
    #[default]
    Idle,
    TargetKeeping = 1,
    ModeCommanding = 2,
}

#[derive(Debug, Default)]
pub enum AssemblyHelperResult {
    #[default]
    Idle,
    TargetKeepingViolation(ComponentId),
    ModeCommandingDone,
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
    pub target_tables: TargetModeTables,
    pub sequence_tables: SequenceModeTables,
    pub helper: SequenceExecutionHelper,
}

impl Default for SubsystemCommandingHelper {
    fn default() -> Self {
        Self {
            current_mode: UNKNOWN_MODE,
            state: Default::default(),
            children_mode_store: Default::default(),
            target_tables: Default::default(),
            sequence_tables: Default::default(),
            helper: Default::default(),
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
            target_tables,
            sequence_tables,
            helper: Default::default(),
        }
    }

    pub fn start_command_sequence(
        &mut self,
        mode: Mode,
        request_id: RequestId,
    ) -> Result<(), ModeDoesNotExistError> {
        self.helper.load(mode, request_id, &self.sequence_tables)?;
        self.state = ModeTreeHelperState::ModeCommanding;
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
                    for entry in &target_table.entries {
                        if !entry.monitor_state {
                            continue;
                        }
                        let mut target_mode_violated = false;
                        self.children_mode_store.0.iter().for_each(|val| {
                            if val.id() == entry.common.target_id {
                                target_mode_violated = if let Some(allowed_submode_mask) =
                                    entry.allowed_submode_mask()
                                {
                                    let fixed_bits = !allowed_submode_mask;
                                    (val.mode_and_submode().mode()
                                        != entry.common.mode_submode.mode())
                                        && (val.mode_and_submode().submode() & fixed_bits
                                            != entry.common.mode_submode.submode() & fixed_bits)
                                } else {
                                    val.mode_and_submode() != entry.common.mode_submode
                                };
                            }
                        })
                    }
                    // Target keeping violated. Report violation and fallback mode to user.
                    return Ok(TargetKeepingResult::Violated {
                        fallback_mode: None,
                    }
                    .into());
                }
                Ok(SubsystemHelperResult::TargetKeeping(
                    TargetKeepingResult::Ok,
                ))
            }
            ModeTreeHelperState::ModeCommanding => {
                let result = self.helper.run(
                    &self.sequence_tables,
                    req_sender,
                    &mut self.children_mode_store,
                )?;
                // By default, the helper will automatically transition into the target keeping
                // mode after an executed sequence.
                if let ModeCommandingResult::CommandingDone = result {
                    self.state = ModeTreeHelperState::TargetKeeping;
                    self.current_mode = ModeAndSubmode::new(self.helper.target_mode().unwrap(), 0);
                }
                Ok(result.into())
            }
        }
    }

    pub fn handle_mode_reply(&mut self, reply: &GenericMessage<ModeReply>) {
        if !self.children_mode_store.has_component(reply.sender_id()) {
            return;
        }
        let mut generic_mode_reply_handler =
            |sender_id, mode_and_submode: Option<ModeAndSubmode>| {
                let still_awating_replies = self
                    .children_mode_store
                    .generic_reply_handler(sender_id, mode_and_submode);
                if self.state == ModeTreeHelperState::ModeCommanding && !still_awating_replies {
                    self.helper.confirm_sequence_done();
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

    pub fn add_target_and_sequence_table(
        &mut self,
        mode: Mode,
        target_table_val: TargetTablesMapValue,
        sequence_table_val: SequenceTablesMapValue,
    ) {
        self.target_tables.0.insert(mode, target_table_val);
        self.sequence_tables.0.insert(mode, sequence_table_val);
    }

    pub fn update_child_mode(
        &mut self,
        child: ComponentId,
        mode: ModeAndSubmode,
    ) -> Result<(), TargetNotInModeStoreError> {
        self.children_mode_store.set_mode(child, mode)
    }
}

#[derive(Default, Debug)]
pub struct ModeRequestHandlerMock {
    get_mode_calls: RefCell<usize>,
    start_transition_calls: VecDeque<(MessageMetadata, ModeAndSubmode)>,
    announce_mode_calls: RefCell<VecDeque<AnnounceModeInfo>>,
    handle_mode_info_calls: VecDeque<(MessageMetadata, ModeAndSubmode)>,
    handle_mode_reached_calls: RefCell<VecDeque<Option<MessageMetadata>>>,
    send_mode_reply_calls: RefCell<VecDeque<(MessageMetadata, ModeReply)>>,
}

impl ModeRequestHandlerMock {
    pub fn clear(&mut self) {
        self.get_mode_calls.replace(0);
        self.start_transition_calls.clear();
        self.announce_mode_calls.borrow_mut().clear();
        self.handle_mode_reached_calls.borrow_mut().clear();
        self.handle_mode_info_calls.clear();
        self.send_mode_reply_calls.borrow_mut().clear();
    }
    pub fn mode_messages_received(&self) -> usize {
        *self.get_mode_calls.borrow()
            + self.start_transition_calls.borrow().len()
            + self.announce_mode_calls.borrow().len()
            + self.handle_mode_info_calls.borrow().len()
            + self.handle_mode_reached_calls.borrow().len()
            + self.send_mode_reply_calls.borrow().len()
    }
}

impl ModeProvider for ModeRequestHandlerMock {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        *self.get_mode_calls.borrow_mut() += 1;
        INVALID_MODE
    }
}

impl ModeRequestHandler for ModeRequestHandlerMock {
    type Error = Infallible;

    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        _forced: bool,
    ) -> Result<(), Self::Error> {
        self.start_transition_calls
            .push_back((requestor, mode_and_submode));
        Ok(())
    }

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool) {
        self.announce_mode_calls
            .borrow_mut()
            .push_back(AnnounceModeInfo {
                requestor: requestor_info,
                recursive,
            });
    }

    fn handle_mode_reached(
        &mut self,
        requestor_info: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        self.handle_mode_reached_calls
            .borrow_mut()
            .push_back(requestor_info);
        Ok(())
    }

    fn handle_mode_info(
        &mut self,
        requestor_info: MessageMetadata,
        info: ModeAndSubmode,
    ) -> Result<(), Self::Error> {
        self.handle_mode_info_calls
            .push_back((requestor_info, info));
        todo!()
    }

    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), Self::Error> {
        self.send_mode_reply_calls
            .borrow_mut()
            .push_back((requestor_info, reply));
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct ModeReplyHandlerMock {
    mode_info_messages: VecDeque<(MessageMetadata, ModeAndSubmode)>,
    mode_reply_messages: VecDeque<(MessageMetadata, ModeAndSubmode)>,
    cant_reach_mode_messages: VecDeque<(MessageMetadata, ResultU16)>,
    wrong_mode_messages: VecDeque<(MessageMetadata, ModeAndSubmode, ModeAndSubmode)>,
}

impl ModeReplyHandlerMock {
    pub fn handle_mode_reply(&mut self, request: &GenericMessage<ModeReply>) {
        match request.message {
            ModeReply::ModeInfo(mode_and_submode) => {
                self.mode_info_messages
                    .push_back((request.requestor_info, mode_and_submode));
            }
            ModeReply::ModeReply(mode_and_submode) => {
                self.mode_reply_messages
                    .push_back((request.requestor_info, mode_and_submode));
            }
            ModeReply::CantReachMode(result_u16) => {
                self.cant_reach_mode_messages
                    .push_back((request.requestor_info, result_u16));
            }
            ModeReply::WrongMode { expected, reached } => {
                self.wrong_mode_messages
                    .push_back((request.requestor_info, expected, reached));
            }
        }
    }
}

struct PusModeService {
    pub request_id_counter: Cell<u32>,
    pub mode_node: ModeRequestorOneChildBoundedMpsc,
}

impl PusModeService {
    pub fn announce_modes_recursively(&self) {
        self.mode_node
            .send_mode_request(
                self.request_id_counter.get(),
                TestComponentId::AcsSubsystem as ComponentId,
                ModeRequest::AnnounceModeRecursive,
            )
            .unwrap();
        self.request_id_counter
            .replace(self.request_id_counter.get() + 1);
    }

    pub fn send_mode_cmd(&self, mode: ModeAndSubmode) {
        self.mode_node
            .send_mode_request(
                self.request_id_counter.get(),
                TestComponentId::AcsSubsystem as ComponentId,
                ModeRequest::SetMode {
                    mode_and_submode: mode,
                    forced: false,
                },
            )
            .unwrap();
        self.request_id_counter
            .replace(self.request_id_counter.get() + 1);
    }
}

impl ModeNode for PusModeService {
    fn id(&self) -> ComponentId {
        TestComponentId::PusModeService as ComponentId
    }
}

impl ModeParent for PusModeService {
    type Sender = RequestSenderType;

    fn add_mode_child(&mut self, id: ComponentId, request_sender: Self::Sender) {
        self.mode_node.add_message_target(id, request_sender);
    }
}

struct AcsSubsystem {
    pub mode_node: ModeRequestorAndHandlerMpscBounded,
    pub mode_requestor_info: Option<MessageMetadata>,
    pub target_mode_and_submode: Option<ModeAndSubmode>,
    pub subsystem_helper: SubsystemCommandingHelper,
    pub mode_req_handler_mock: ModeRequestHandlerMock,
    pub mode_req_recvd: u32,
}

impl AcsSubsystem {
    pub fn new(mode_node: ModeRequestorAndHandlerMpscBounded) -> Self {
        Self {
            mode_node,
            mode_requestor_info: None,
            target_mode_and_submode: None,
            subsystem_helper: SubsystemCommandingHelper::default(),
            mode_req_handler_mock: Default::default(),
            mode_req_recvd: 0,
        }
    }

    pub fn get_and_clear_num_mode_requests(&mut self) -> u32 {
        let tmp = self.mode_req_recvd;
        self.mode_req_recvd = 0;
        tmp
    }

    pub fn run(&mut self) {
        if let Some(request) = self.mode_node.try_recv_mode_request().unwrap() {
            self.mode_req_recvd += 1;
            self.handle_mode_request(request)
                .expect("mode messaging error");
        }
        let mut mode_reply = None;
        if let Some(reply) = self.mode_node.try_recv_mode_reply().unwrap() {
            mode_reply = Some(reply);
        }
        match self
            .subsystem_helper
            .state_machine(mode_reply, &self.mode_node)
        {
            Ok(result) => match result {
                SubsystemHelperResult::Idle => (),
                SubsystemHelperResult::TargetKeeping(target_keeping_result) => {
                    match target_keeping_result {
                        TargetKeepingResult::Ok => todo!(),
                        TargetKeepingResult::Violated { fallback_mode } => {
                            if let Some(fallback_mode) = fallback_mode {
                                self.subsystem_helper
                                    .start_command_sequence(fallback_mode, 0)
                                    .unwrap();
                            }
                        }
                    }
                }
                SubsystemHelperResult::ModeCommanding(sequence_handler_result) => {
                    match sequence_handler_result {
                        ModeCommandingResult::CommandingDone => (),
                        ModeCommandingResult::CommandingStepDone => (),
                        ModeCommandingResult::AwaitingSuccessCheck => (),
                    }
                }
            },
            Err(error) => match error {
                ModeTreeHelperError::Message(_generic_targeted_messaging_error) => {
                    panic!("messaging error")
                }
                ModeTreeHelperError::CurrentModeNotInTargetTable(_) => panic!("mode not found"),
            },
        }
    }
    pub fn add_target_and_sequence_table(
        &mut self,
        mode: Mode,
        target_table_val: TargetTablesMapValue,
        sequence_table_val: SequenceTablesMapValue,
    ) {
        self.subsystem_helper.add_target_and_sequence_table(
            mode,
            target_table_val,
            sequence_table_val,
        );
    }
}

impl ModeNode for AcsSubsystem {
    fn id(&self) -> ComponentId {
        TestComponentId::AcsSubsystem as ComponentId
    }
}

impl ModeParent for AcsSubsystem {
    type Sender = RequestSenderType;

    fn add_mode_child(&mut self, id: ComponentId, request_sender: RequestSenderType) {
        self.subsystem_helper
            .children_mode_store
            .add_component(id, UNKNOWN_MODE);
        self.mode_node.add_request_target(id, request_sender);
    }
}

impl ModeChild for AcsSubsystem {
    type Sender = ReplySenderType;

    fn add_mode_parent(&mut self, id: ComponentId, reply_sender: ReplySenderType) {
        self.mode_node.add_reply_target(id, reply_sender);
    }
}

impl ModeProvider for AcsSubsystem {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.subsystem_helper.current_mode
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubsystemModeError {
    #[error("messaging error: {0:?}")]
    Mode(#[from] ModeError),
    #[error("mode does not exist: {0}")]
    ModeDoesNotExist(#[from] ModeDoesNotExistError),
}

impl ModeRequestHandler for AcsSubsystem {
    type Error = SubsystemModeError;

    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
    ) -> Result<(), Self::Error> {
        if !forced && self.subsystem_helper.state == ModeTreeHelperState::ModeCommanding {
            return Err(ModeError::Busy.into());
        }
        self.mode_requestor_info = Some(requestor);
        self.target_mode_and_submode = Some(mode_and_submode);
        self.mode_req_handler_mock
            .start_transition(requestor, mode_and_submode, forced)
            .unwrap();
        self.subsystem_helper
            .start_command_sequence(mode_and_submode.mode(), requestor.request_id())?;
        Ok(())
    }

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool) {
        println!(
            "TestAssembly: Announcing mode (recursively: {}): {:?}",
            recursive, self.subsystem_helper.current_mode
        );
        let mut mode_request = ModeRequest::AnnounceMode;
        if recursive {
            mode_request = ModeRequest::AnnounceModeRecursive;
        }
        let request_id = requestor_info.map_or(0, |info| info.request_id());
        self.mode_node
            .request_sender_store
            .0
            .iter()
            .for_each(|(_, sender)| {
                sender
                    .send(GenericMessage::new(
                        MessageMetadata::new(request_id, self.mode_node.local_channel_id_generic()),
                        mode_request,
                    ))
                    .expect("sending mode request failed");
            });
        self.mode_req_handler_mock
            .announce_mode(requestor_info, recursive);
    }

    fn handle_mode_reached(
        &mut self,
        requestor_info: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        if let Some(requestor) = requestor_info {
            self.send_mode_reply(
                requestor,
                ModeReply::ModeReply(self.subsystem_helper.current_mode),
            )?;
        }
        self.mode_req_handler_mock
            .handle_mode_reached(requestor_info)
            .unwrap();
        Ok(())
    }

    fn handle_mode_info(
        &mut self,
        requestor_info: MessageMetadata,
        info: ModeAndSubmode,
    ) -> Result<(), Self::Error> {
        self.mode_req_handler_mock
            .handle_mode_info(requestor_info, info)
            .unwrap();
        // TODO: Need to check whether mode table execution is finished.
        // This works by checking the children modes received through replies against the
        // mode table after all transition tables were executed.
        Ok(())
    }

    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), Self::Error> {
        self.mode_req_handler_mock
            .send_mode_reply(requestor_info, reply)
            .unwrap();
        self.mode_node
            .send_mode_reply(requestor_info, reply)
            .map_err(ModeError::Messaging)?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct AssemblyCommandingHelper {
    /// The IDs, modes and reply awaition status of all children are tracked in this data
    /// structure.
    pub children_mode_store: ModeStoreVec,
    /// Target mode used for mode commanding.
    pub target_mode: Option<ModeAndSubmode>,
    pub state: ModeTreeHelperState,
}

impl AssemblyCommandingHelper {
    pub fn send_mode_cmd_to_all_children_with_reply_awaition(
        &mut self,
        request_id: RequestId,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
        mode_req_sender: &impl ModeRequestSender,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.target_mode = Some(mode_and_submode);
        for child in self.children_mode_store.0.iter_mut() {
            mode_req_sender.send_mode_request(
                request_id,
                child.id(),
                ModeRequest::SetMode {
                    mode_and_submode,
                    forced,
                },
            )?;
            child.awaiting_reply = true;
        }
        Ok(())
    }

    pub fn count_number_of_children_with_target_mode(&self) -> Option<usize> {
        self.target_mode?;
        let target_mode = self.target_mode.unwrap();
        let mut children_in_target_mode = 0;
        for child in self.children_mode_store.0.iter() {
            if child.mode_and_submode() == target_mode {
                children_in_target_mode += 1;
            }
        }
        Some(children_in_target_mode)
    }

    pub fn handle_mode_reply(
        &mut self,
        mode_reply: &GenericMessage<ModeReply>,
    ) -> AssemblyHelperResult {
        if !self
            .children_mode_store
            .has_component(mode_reply.sender_id())
        {
            return AssemblyHelperResult::Idle;
        }
        let mut generic_mode_reply_handler = |mode_and_submode: Option<ModeAndSubmode>| {
            let still_awating_replies = self
                .children_mode_store
                .generic_reply_handler(mode_reply.sender_id(), mode_and_submode);
            if self.state == ModeTreeHelperState::TargetKeeping
                && mode_and_submode.is_some()
                && self.target_mode.is_some()
                && mode_and_submode.unwrap() != self.target_mode.unwrap()
            {
                return AssemblyHelperResult::TargetKeepingViolation(mode_reply.sender_id());
            }
            if self.state == ModeTreeHelperState::ModeCommanding && !still_awating_replies {
                self.state = ModeTreeHelperState::TargetKeeping;
                return AssemblyHelperResult::ModeCommandingDone;
            }
            AssemblyHelperResult::Idle
        };
        match mode_reply.message {
            ModeReply::ModeInfo(mode_and_submode) | ModeReply::ModeReply(mode_and_submode) => {
                generic_mode_reply_handler(Some(mode_and_submode))
            }
            ModeReply::CantReachMode(_result_u16) => generic_mode_reply_handler(None),
            ModeReply::WrongMode {
                expected: _,
                reached,
            } => generic_mode_reply_handler(Some(reached)),
        }
    }
}

// TODO: This assembly requires some helper component to process commands.. Maybe implement it
// manually first?
struct MgmAssembly {
    pub mode_node: ModeRequestorAndHandlerMpscBounded,
    pub mode_requestor_info: Option<MessageMetadata>,
    pub mode_and_submode: ModeAndSubmode,
    pub assembly_helper: AssemblyCommandingHelper,
    pub mode_req_mock: ModeRequestHandlerMock,
    pub mode_reply_mock: ModeReplyHandlerMock,
}

impl MgmAssembly {
    pub fn new(mode_node: ModeRequestorAndHandlerMpscBounded) -> Self {
        Self {
            mode_node,
            mode_requestor_info: None,
            mode_and_submode: UNKNOWN_MODE,
            assembly_helper: Default::default(),
            mode_req_mock: Default::default(),
            mode_reply_mock: Default::default(),
        }
    }

    pub fn run(&mut self) {
        self.check_mode_requests().expect("mode messaging error");
        self.check_mode_replies().expect("mode messaging error");
    }
    pub fn get_num_mode_requests(&mut self) -> usize {
        self.mode_req_mock.mode_messages_received()
    }

    pub fn check_mode_requests(&mut self) -> Result<(), GenericTargetedMessagingError> {
        if let Some(request) = self.mode_node.try_recv_mode_request()? {
            self.handle_mode_request(request).unwrap();
        }
        Ok(())
    }

    pub fn check_mode_replies(&mut self) -> Result<(), ModeError> {
        if let Some(reply_and_id) = self.mode_node.try_recv_mode_reply()? {
            self.mode_reply_mock.handle_mode_reply(&reply_and_id);
            match self.assembly_helper.handle_mode_reply(&reply_and_id) {
                AssemblyHelperResult::Idle => (),
                AssemblyHelperResult::TargetKeepingViolation(_id) => {
                    // TODO: Check whether enough children are available to keep the mode.
                    // Otherwise, we command everything OFF, because we can not keep the mode.
                }
                AssemblyHelperResult::ModeCommandingDone => {
                    if self.assembly_helper.target_mode.is_some() {
                        // Complete the mode command.
                        self.handle_mode_reached(self.mode_requestor_info)?;
                        self.mode_and_submode = self.assembly_helper.target_mode.take().unwrap();
                    }
                }
            }
        }
        Ok(())
    }
}

impl ModeNode for MgmAssembly {
    fn id(&self) -> ComponentId {
        TestComponentId::MagnetometerAssembly as u64
    }
}
impl ModeParent for MgmAssembly {
    type Sender = RequestSenderType;

    fn add_mode_child(&mut self, id: ComponentId, request_sender: RequestSenderType) {
        self.mode_node.add_request_target(id, request_sender);
    }
}

impl ModeChild for MgmAssembly {
    type Sender = ReplySenderType;

    fn add_mode_parent(&mut self, id: ComponentId, reply_sender: ReplySenderType) {
        self.mode_node.add_reply_target(id, reply_sender);
    }
}

impl ModeProvider for MgmAssembly {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl ModeRequestHandler for MgmAssembly {
    type Error = ModeError;
    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
    ) -> Result<(), Self::Error> {
        // Always accept forced commands and commands to mode OFF.
        if self.assembly_helper.target_mode.is_some()
            && !forced
            && mode_and_submode.mode() != DefaultMode::OFF as u32
        {
            return Err(ModeError::Busy);
        }
        self.mode_requestor_info = Some(requestor);
        self.mode_req_mock
            .start_transition(requestor, mode_and_submode, forced)
            .unwrap();
        self.assembly_helper
            .send_mode_cmd_to_all_children_with_reply_awaition(
                requestor.request_id(),
                mode_and_submode,
                forced,
                &self.mode_node,
            )?;
        Ok(())
    }

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool) {
        println!(
            "TestAssembly: Announcing mode (recursively: {}): {:?}",
            recursive, self.mode_and_submode
        );
        let mut mode_request = ModeRequest::AnnounceMode;
        if recursive {
            mode_request = ModeRequest::AnnounceModeRecursive;
        }
        let request_id = requestor_info.map_or(0, |info| info.request_id());
        self.mode_node
            .request_sender_store
            .0
            .iter()
            .for_each(|(_, sender)| {
                sender
                    .send(GenericMessage::new(
                        MessageMetadata::new(request_id, self.mode_node.local_channel_id_generic()),
                        mode_request,
                    ))
                    .expect("sending mode request failed");
            });
        self.mode_req_mock.announce_mode(requestor_info, recursive);
    }

    fn handle_mode_reached(
        &mut self,
        mode_requestor: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        if let Some(requestor) = mode_requestor {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
        }
        self.mode_req_mock
            .handle_mode_reached(mode_requestor)
            .unwrap();
        Ok(())
    }

    fn handle_mode_info(
        &mut self,
        requestor_info: MessageMetadata,
        info: ModeAndSubmode,
    ) -> Result<(), Self::Error> {
        self.mode_req_mock
            .handle_mode_info(requestor_info, info)
            .unwrap();
        // TODO: A proper assembly must reach to mode changes of its children..
        Ok(())
    }

    fn send_mode_reply(
        &self,
        requestor: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), Self::Error> {
        self.mode_node.send_mode_reply(requestor, reply)?;
        self.mode_req_mock
            .send_mode_reply(requestor, reply)
            .unwrap();
        Ok(())
    }
}

struct DeviceManager {
    name: &'static str,
    pub id: ComponentId,
    pub mode_node: ModeRequestorAndHandlerMpscBounded,
    pub mode_and_submode: ModeAndSubmode,
    pub mode_req_mock: ModeRequestHandlerMock,
    pub mode_req_recvd: u32,
}

impl DeviceManager {
    pub fn new(
        name: &'static str,
        id: ComponentId,
        mode_node: ModeRequestorAndHandlerMpscBounded,
    ) -> Self {
        Self {
            name,
            id,
            mode_node,
            mode_and_submode: UNKNOWN_MODE,
            mode_req_mock: Default::default(),
            mode_req_recvd: 0,
        }
    }

    pub fn get_and_clear_num_mode_requests(&mut self) -> u32 {
        let tmp = self.mode_req_recvd;
        self.mode_req_recvd = 0;
        tmp
    }

    pub fn run(&mut self) {
        self.check_mode_requests().expect("mode messaging error");
    }

    pub fn check_mode_requests(&mut self) -> Result<(), ModeError> {
        if let Some(request) = self.mode_node.try_recv_mode_request()? {
            self.mode_req_recvd += 1;
            self.handle_mode_request(request)?
        }
        Ok(())
    }
}

impl ModeNode for DeviceManager {
    fn id(&self) -> ComponentId {
        self.id
    }
}

impl ModeChild for DeviceManager {
    type Sender = ReplySenderType;

    fn add_mode_parent(&mut self, id: ComponentId, reply_sender: ReplySenderType) {
        self.mode_node.add_reply_target(id, reply_sender);
    }
}

impl ModeParent for DeviceManager {
    type Sender = RequestSenderType;

    fn add_mode_child(&mut self, id: ComponentId, request_sender: Self::Sender) {
        self.mode_node.add_request_target(id, request_sender);
    }
}

impl ModeProvider for DeviceManager {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl ModeRequestHandler for DeviceManager {
    type Error = ModeError;

    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
    ) -> Result<(), ModeError> {
        self.mode_and_submode = mode_and_submode;
        self.handle_mode_reached(Some(requestor))?;
        self.mode_req_mock
            .start_transition(requestor, mode_and_submode, forced)
            .unwrap();
        Ok(())
    }

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool) {
        println!(
            "{}: announcing mode: {:?}",
            self.name, self.mode_and_submode
        );
        let mut mode_request = ModeRequest::AnnounceMode;
        if recursive {
            mode_request = ModeRequest::AnnounceModeRecursive;
        }
        let request_id = requestor_info.map_or(0, |info| info.request_id());
        self.mode_node
            .request_sender_store
            .0
            .iter()
            .for_each(|(_, sender)| {
                sender
                    .send(GenericMessage::new(
                        MessageMetadata::new(request_id, self.mode_node.local_channel_id_generic()),
                        mode_request,
                    ))
                    .expect("sending mode request failed");
            });
        self.mode_req_mock.announce_mode(requestor_info, recursive);
    }

    fn handle_mode_reached(&mut self, requestor: Option<MessageMetadata>) -> Result<(), ModeError> {
        if let Some(requestor) = requestor {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
        }
        self.mode_req_mock.handle_mode_reached(requestor).unwrap();
        Ok(())
    }
    fn handle_mode_info(
        &mut self,
        requestor_info: MessageMetadata,
        info: ModeAndSubmode,
    ) -> Result<(), ModeError> {
        // A device is a leaf in the tree.. so this really should not happen
        println!(
            "{}: unexpected mode info from {:?} with mode: {:?}",
            self.name,
            requestor_info.sender_id(),
            info
        );
        self.mode_req_mock
            .handle_mode_info(requestor_info, info)
            .unwrap();
        Ok(())
    }

    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), ModeError> {
        self.mode_node.send_mode_reply(requestor_info, reply)?;
        self.mode_req_mock
            .send_mode_reply(requestor_info, reply)
            .unwrap();
        Ok(())
    }
}

struct CommonDevice {
    name: &'static str,
    pub id: ComponentId,
    pub mode_node: ModeRequestHandlerMpscBounded,
    pub mode_and_submode: ModeAndSubmode,
    pub mode_req_mock: ModeRequestHandlerMock,
    pub num_mode_msgs_recvd: u32,
}

impl CommonDevice {
    pub fn new(
        name: &'static str,
        id: ComponentId,
        mode_node: ModeRequestHandlerMpscBounded,
    ) -> Self {
        Self {
            name,
            id,
            mode_node,
            mode_and_submode: UNKNOWN_MODE,
            mode_req_mock: Default::default(),
            num_mode_msgs_recvd: 0,
        }
    }

    pub fn run(&mut self) {
        self.check_mode_requests().expect("mode messaging error");
    }

    pub fn check_mode_requests(&mut self) -> Result<(), ModeError> {
        if let Some(request) = self.mode_node.try_recv_mode_request()? {
            self.num_mode_msgs_recvd += 1;
            self.handle_mode_request(request)?
        }
        Ok(())
    }

    pub fn get_and_clear_num_mode_msgs(&mut self) -> u32 {
        let tmp = self.num_mode_msgs_recvd;
        self.num_mode_msgs_recvd = 0;
        tmp
    }
}

impl ModeNode for CommonDevice {
    fn id(&self) -> ComponentId {
        self.id
    }
}

impl ModeChild for CommonDevice {
    type Sender = ReplySenderType;

    fn add_mode_parent(&mut self, id: ComponentId, reply_sender: ReplySenderType) {
        self.mode_node.add_message_target(id, reply_sender);
    }
}

impl ModeProvider for CommonDevice {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl ModeRequestHandler for CommonDevice {
    type Error = ModeError;

    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
    ) -> Result<(), ModeError> {
        self.mode_and_submode = mode_and_submode;
        self.handle_mode_reached(Some(requestor))?;
        self.mode_req_mock
            .start_transition(requestor, mode_and_submode, forced)
            .unwrap();
        Ok(())
    }

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool) {
        println!(
            "{}: announcing mode: {:?}",
            self.name, self.mode_and_submode
        );
        self.mode_req_mock.announce_mode(requestor_info, recursive);
    }

    fn handle_mode_reached(&mut self, requestor: Option<MessageMetadata>) -> Result<(), ModeError> {
        if let Some(requestor) = requestor {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
        }
        self.mode_req_mock.handle_mode_reached(requestor).unwrap();
        Ok(())
    }

    fn handle_mode_info(
        &mut self,
        requestor_info: MessageMetadata,
        info: ModeAndSubmode,
    ) -> Result<(), ModeError> {
        // A device is a leaf in the tree.. so this really should not happen
        println!(
            "{}: unexpected mode info from {:?} with mode: {:?}",
            self.name,
            requestor_info.sender_id(),
            info
        );
        self.mode_req_mock
            .handle_mode_info(requestor_info, info)
            .unwrap();
        Ok(())
    }

    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), ModeError> {
        self.mode_node.send_mode_reply(requestor_info, reply)?;
        self.mode_req_mock
            .send_mode_reply(requestor_info, reply)
            .unwrap();
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct AnnounceModeInfo {
    pub requestor: Option<MessageMetadata>,
    pub recursive: bool,
}

pub struct AcsController {
    pub mode_node: ModeRequestHandlerMpscBounded,
    pub mode_and_submode: ModeAndSubmode,
    pub announce_mode_queue: RefCell<VecDeque<AnnounceModeInfo>>,
    pub mode_req_mock: ModeRequestHandlerMock,
}

impl AcsController {
    pub fn new(mode_node: ModeRequestHandlerMpscBounded) -> Self {
        Self {
            mode_node,
            mode_and_submode: UNKNOWN_MODE,
            announce_mode_queue: Default::default(),
            mode_req_mock: Default::default(),
        }
    }
    pub fn run(&mut self) {
        self.check_mode_requests().expect("mode messaging error");
    }

    pub fn check_mode_requests(&mut self) -> Result<(), ModeError> {
        if let Some(request) = self.mode_node.try_recv_mode_request()? {
            self.handle_mode_request(request)?
        }
        Ok(())
    }
}

impl ModeNode for AcsController {
    fn id(&self) -> ComponentId {
        TestComponentId::AcsController as u64
    }
}

impl ModeChild for AcsController {
    type Sender = ReplySenderType;

    fn add_mode_parent(&mut self, id: ComponentId, reply_sender: ReplySenderType) {
        self.mode_node.add_message_target(id, reply_sender);
    }
}

impl ModeProvider for AcsController {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl ModeRequestHandler for AcsController {
    type Error = ModeError;

    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
    ) -> Result<(), Self::Error> {
        self.mode_and_submode = mode_and_submode;
        self.handle_mode_reached(Some(requestor))?;
        self.mode_req_mock
            .start_transition(requestor, mode_and_submode, forced)
            .unwrap();
        Ok(())
    }

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool) {
        self.announce_mode_queue
            .borrow_mut()
            .push_back(AnnounceModeInfo {
                requestor: requestor_info,
                recursive,
            });
        self.mode_req_mock.announce_mode(requestor_info, recursive);
    }

    fn handle_mode_reached(
        &mut self,
        requestor_info: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        if let Some(requestor) = requestor_info {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
        }
        self.mode_req_mock
            .handle_mode_reached(requestor_info)
            .unwrap();
        Ok(())
    }

    fn handle_mode_info(
        &mut self,
        requestor_info: MessageMetadata,
        info: ModeAndSubmode,
    ) -> Result<(), Self::Error> {
        // The controller is a leaf in the tree.. so this really should not happen
        println!(
            "ACS Controller: unexpected mode info from {:?} with mode: {:?}",
            requestor_info.sender_id(),
            info
        );
        self.mode_req_mock
            .handle_mode_info(requestor_info, info)
            .unwrap();
        Ok(())
    }

    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), Self::Error> {
        self.mode_node.send_mode_reply(requestor_info, reply)?;
        self.mode_req_mock
            .send_mode_reply(requestor_info, reply)
            .unwrap();
        Ok(())
    }
}

pub struct TreeTestbench {
    pus: PusModeService,
    subsystem: AcsSubsystem,
    ctrl: AcsController,
    mgt_manager: DeviceManager,
    mgm_assy: MgmAssembly,
    mgm_devs: [CommonDevice; 2],
    mgt_dev: CommonDevice,
}

impl Default for TreeTestbench {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeTestbench {
    pub fn new() -> Self {
        // All request channel handles.
        let (request_sender_to_mgm_dev_0, request_receiver_mgm_dev_0) = mpsc::sync_channel(10);
        let (request_sender_to_mgm_dev_1, request_receiver_mgm_dev_1) = mpsc::sync_channel(10);
        let (request_sender_to_mgt_dev, request_receiver_mgt_dev) = mpsc::sync_channel(10);
        let (request_sender_to_mgt_man, request_receiver_mgt_man) = mpsc::sync_channel(10);
        let (request_sender_to_mgm_assy, request_receiver_mgm_assy) = mpsc::sync_channel(10);
        let (request_sender_to_acs_subsystem, request_receiver_acs_subsystem) =
            mpsc::sync_channel(10);
        let (request_sender_to_acs_ctrl, request_receiver_acs_ctrl) = mpsc::sync_channel(10);

        // All reply channel handles.
        let (reply_sender_to_mgm_assy, reply_receiver_mgm_assy) = mpsc::sync_channel(10);
        let (reply_sender_to_acs_subsystem, reply_receiver_acs_subsystem) = mpsc::sync_channel(10);
        let (reply_sender_to_pus, reply_receiver_pus) = mpsc::sync_channel(10);
        let (reply_sender_to_mgt_man, reply_receiver_mgt_man) = mpsc::sync_channel(10);

        // Mode requestors only.
        let mode_node_pus = ModeRequestorOneChildBoundedMpsc::new(
            TestComponentId::PusModeService as ComponentId,
            reply_receiver_pus,
        );

        // Mode requestors and handlers.
        let mgm_assy_node = ModeRequestorAndHandlerMpscBounded::new(
            TestComponentId::MagnetometerAssembly as ComponentId,
            request_receiver_mgm_assy,
            reply_receiver_mgm_assy,
        );
        let mgt_dev_mgmt_node = ModeRequestorAndHandlerMpscBounded::new(
            TestComponentId::MgtDevManager as ComponentId,
            request_receiver_mgt_man,
            reply_receiver_mgt_man,
        );
        let acs_subsystem_node = ModeRequestorAndHandlerMpscBounded::new(
            TestComponentId::AcsSubsystem as ComponentId,
            request_receiver_acs_subsystem,
            reply_receiver_acs_subsystem,
        );

        // Request handlers only.
        let mgm_dev_node_0 = ModeRequestHandlerMpscBounded::new(
            TestComponentId::MagnetometerDevice0 as ComponentId,
            request_receiver_mgm_dev_0,
        );
        let mgm_dev_node_1 = ModeRequestHandlerMpscBounded::new(
            TestComponentId::MagnetometerDevice1 as ComponentId,
            request_receiver_mgm_dev_1,
        );
        let mgt_dev_node = ModeRequestHandlerMpscBounded::new(
            TestComponentId::MagnetorquerDevice as ComponentId,
            request_receiver_mgt_dev,
        );
        let acs_ctrl_node = ModeRequestHandlerMpscBounded::new(
            TestComponentId::AcsController as ComponentId,
            request_receiver_acs_ctrl,
        );

        let mut mgm_dev_0 = CommonDevice::new(
            "MGM_0",
            TestComponentId::MagnetometerDevice0 as u64,
            mgm_dev_node_0,
        );
        let mut mgm_dev_1 = CommonDevice::new(
            "MGM_1",
            TestComponentId::MagnetometerDevice1 as u64,
            mgm_dev_node_1,
        );
        let mut mgt_dev = CommonDevice::new(
            "MGT",
            TestComponentId::MagnetorquerDevice as u64,
            mgt_dev_node,
        );
        let mut mgt_manager = DeviceManager::new(
            "MGT_MANAGER",
            TestComponentId::MgtDevManager as u64,
            mgt_dev_mgmt_node,
        );
        let mut mgm_assy = MgmAssembly::new(mgm_assy_node);
        let mut acs_subsystem = AcsSubsystem::new(acs_subsystem_node);
        let mut acs_ctrl = AcsController::new(acs_ctrl_node);
        let mut pus_service = PusModeService {
            request_id_counter: Cell::new(0),
            mode_node: mode_node_pus,
        };

        // ACS subsystem tables
        let mut target_table_safe = TargetTablesMapValue::new("SAFE_TARGET_TBL", None);
        target_table_safe.add_entry(TargetTableEntry::new(
            "CTRL_SAFE",
            TestComponentId::AcsController as u64,
            ModeAndSubmode::new(AcsMode::SAFE as u32, 0),
            // All submodes allowed.
            Some(0xffff),
        ));
        target_table_safe.add_entry(TargetTableEntry::new_with_precise_submode(
            "MGM_A_NML",
            TestComponentId::MagnetometerAssembly as u64,
            ModeAndSubmode::new(DefaultMode::NORMAL as u32, 0),
        ));
        target_table_safe.add_entry(TargetTableEntry::new_with_precise_submode(
            "MGT_MAN_NML",
            TestComponentId::MgtDevManager as u64,
            ModeAndSubmode::new(DefaultMode::NORMAL as u32, 0),
        ));
        let mut sequence_tbl_safe_0 = SequenceTableMapTable::new("SAFE_SEQ_0_TBL");
        sequence_tbl_safe_0.add_entry(SequenceTableEntry::new(
            "SAFE_SEQ_0_MGM_A",
            TestComponentId::MagnetometerAssembly as u64,
            ModeAndSubmode::new(DefaultMode::NORMAL as Mode, 0),
            false,
        ));
        sequence_tbl_safe_0.add_entry(SequenceTableEntry::new(
            "SAFE_SEQ_0_MGT_MAN",
            TestComponentId::MgtDevManager as u64,
            ModeAndSubmode::new(DefaultMode::NORMAL as Mode, 0),
            false,
        ));
        let mut sequence_tbl_safe_1 = SequenceTableMapTable::new("SAFE_SEQ_1_TBL");
        sequence_tbl_safe_1.add_entry(SequenceTableEntry::new(
            "SAFE_SEQ_1_ACS_CTRL",
            TestComponentId::AcsController as u64,
            ModeAndSubmode::new(AcsMode::SAFE as Mode, 0),
            false,
        ));
        let mut sequence_tbl_safe = SequenceTablesMapValue::new("SAFE_SEQ_TBL");
        sequence_tbl_safe.add_sequence_table(sequence_tbl_safe_0);
        sequence_tbl_safe.add_sequence_table(sequence_tbl_safe_1);
        acs_subsystem.add_target_and_sequence_table(
            AcsMode::SAFE as u32,
            target_table_safe,
            sequence_tbl_safe,
        );

        // Connect the PUS mode service to all mode objects.
        connect_mode_nodes(
            &mut pus_service,
            request_sender_to_acs_subsystem,
            &mut acs_subsystem,
            reply_sender_to_pus.clone(),
        );
        connect_mode_nodes(
            &mut pus_service,
            request_sender_to_acs_ctrl.clone(),
            &mut acs_ctrl,
            reply_sender_to_pus.clone(),
        );
        connect_mode_nodes(
            &mut pus_service,
            request_sender_to_mgm_dev_0.clone(),
            &mut mgm_dev_0,
            reply_sender_to_pus.clone(),
        );
        connect_mode_nodes(
            &mut pus_service,
            request_sender_to_mgm_dev_1.clone(),
            &mut mgm_dev_1,
            reply_sender_to_pus.clone(),
        );
        connect_mode_nodes(
            &mut pus_service,
            request_sender_to_mgm_assy.clone(),
            &mut mgm_assy,
            reply_sender_to_pus.clone(),
        );
        connect_mode_nodes(
            &mut pus_service,
            request_sender_to_mgt_man.clone(),
            &mut mgt_manager,
            reply_sender_to_pus.clone(),
        );
        connect_mode_nodes(
            &mut pus_service,
            request_sender_to_mgt_dev.clone(),
            &mut mgt_dev,
            reply_sender_to_pus.clone(),
        );

        // Connect the ACS subsystem to all children.
        connect_mode_nodes(
            &mut acs_subsystem,
            request_sender_to_mgm_assy,
            &mut mgm_assy,
            reply_sender_to_acs_subsystem.clone(),
        );
        connect_mode_nodes(
            &mut acs_subsystem,
            request_sender_to_acs_ctrl,
            &mut acs_ctrl,
            reply_sender_to_acs_subsystem.clone(),
        );
        connect_mode_nodes(
            &mut acs_subsystem,
            request_sender_to_mgt_man.clone(),
            &mut mgt_manager,
            reply_sender_to_acs_subsystem.clone(),
        );

        connect_mode_nodes(
            &mut mgm_assy,
            request_sender_to_mgm_dev_0,
            &mut mgm_dev_0,
            reply_sender_to_mgm_assy.clone(),
        );
        connect_mode_nodes(
            &mut mgm_assy,
            request_sender_to_mgm_dev_1,
            &mut mgm_dev_1,
            reply_sender_to_mgm_assy,
        );

        connect_mode_nodes(
            &mut mgt_manager,
            request_sender_to_mgt_dev,
            &mut mgt_dev,
            reply_sender_to_mgt_man,
        );
        Self {
            pus: pus_service,
            subsystem: acs_subsystem,
            ctrl: acs_ctrl,
            mgm_assy,
            mgt_dev,
            mgt_manager,
            mgm_devs: [mgm_dev_0, mgm_dev_1],
        }
    }

    pub fn run(&mut self) {
        self.subsystem.run();
        self.ctrl.run();
        self.mgt_manager.run();
        self.mgm_assy.run();
        self.mgm_devs[0].run();
        self.mgm_devs[1].run();
        self.mgt_dev.run();
    }
}

#[test]
fn announce_recursively() {
    let mut tb = TreeTestbench::new();
    tb.pus.announce_modes_recursively();
    // Run everything twice so the order does not matter.
    tb.run();
    tb.run();
    assert_eq!(tb.subsystem.get_and_clear_num_mode_requests(), 1);
    let mut announces = tb
        .subsystem
        .mode_req_handler_mock
        .announce_mode_calls
        .borrow_mut();
    assert_eq!(announces.len(), 1);
    announces = tb.ctrl.mode_req_mock.announce_mode_calls.borrow_mut();
    assert_eq!(tb.ctrl.mode_req_mock.start_transition_calls.len(), 0);
    assert_eq!(tb.ctrl.mode_and_submode(), UNKNOWN_MODE);
    assert_eq!(announces.len(), 1);
    assert_eq!(tb.mgm_assy.get_num_mode_requests(), 1);
    announces = tb.mgm_assy.mode_req_mock.announce_mode_calls.borrow_mut();
    assert_eq!(tb.mgm_assy.mode_req_mock.start_transition_calls.len(), 0);
    assert_eq!(tb.mgm_assy.mode_and_submode(), UNKNOWN_MODE);
    assert_eq!(announces.len(), 1);
    for mgm_dev in &mut tb.mgm_devs {
        assert_eq!(mgm_dev.get_and_clear_num_mode_msgs(), 1);
        announces = mgm_dev.mode_req_mock.announce_mode_calls.borrow_mut();
        assert_eq!(mgm_dev.mode_req_mock.start_transition_calls.len(), 0);
        assert_eq!(mgm_dev.mode_and_submode(), UNKNOWN_MODE);
        assert_eq!(announces.len(), 1);
    }
    assert_eq!(announces.len(), 1);
    assert_eq!(tb.mgt_dev.get_and_clear_num_mode_msgs(), 1);
    announces = tb.mgt_dev.mode_req_mock.announce_mode_calls.borrow_mut();
    assert_eq!(tb.mgt_dev.mode_req_mock.start_transition_calls.len(), 0);
    assert_eq!(tb.mgt_dev.mode_and_submode(), UNKNOWN_MODE);
    assert_eq!(announces.len(), 1);
    assert_eq!(tb.mgt_manager.get_and_clear_num_mode_requests(), 1);
    announces = tb
        .mgt_manager
        .mode_req_mock
        .announce_mode_calls
        .borrow_mut();
    assert_eq!(tb.mgt_manager.mode_req_mock.start_transition_calls.len(), 0);
    assert_eq!(tb.mgt_manager.mode_and_submode(), UNKNOWN_MODE);
    assert_eq!(announces.len(), 1);
}

#[test]
fn command_safe_mode() {
    let mut tb = TreeTestbench::new();
    tb.run();
    tb.pus
        .send_mode_cmd(ModeAndSubmode::new(AcsMode::SAFE as u32, 0));
    tb.run();
    tb.run();
    assert_eq!(
        tb.ctrl.mode_and_submode(),
        ModeAndSubmode::new(AcsMode::SAFE as u32, 0)
    );
    assert_eq!(
        tb.mgm_assy.mode_and_submode(),
        ModeAndSubmode::new(DefaultMode::NORMAL as u32, 0)
    );
}
