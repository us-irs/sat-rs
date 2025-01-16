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
use satrs::subsystem::{
    ModeDoesNotExistError, SequenceExecutionHelper, SequenceHandlerResult, TargetKeepingResult,
};
use satrs::{
    mode::{ModeAndSubmode, ModeReply, ModeRequest},
    queue::GenericTargetedMessagingError,
    request::GenericMessage,
    ComponentId,
};
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

#[derive(Debug, Default)]
pub enum ModeTreeHelperState {
    #[default]
    Idle,
    TargetKeeping = 1,
    SequenceCommanding = 2,
}

#[derive(Debug)]
pub enum ModeTreeHelperResult {
    Idle,
    TargetKeeping(TargetKeepingResult),
    SequenceCommanding(SequenceHandlerResult),
}

impl From<TargetKeepingResult> for ModeTreeHelperResult {
    fn from(value: TargetKeepingResult) -> Self {
        Self::TargetKeeping(value)
    }
}

impl From<SequenceHandlerResult> for ModeTreeHelperResult {
    fn from(value: SequenceHandlerResult) -> Self {
        Self::SequenceCommanding(value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModeTreeHelperError {
    #[error("generic targeted messaging error: {0}")]
    Message(#[from] GenericTargetedMessagingError),
    #[error("current mode {0} is not contained in target table")]
    CurrentModeNotInTargetTable(Mode),
}

// TODO:
//
// 1. Fallback mode? Needs to be a part of the target mode table..
// 2. State to determine whether we are in sequence execution mode or in target keeping mode.
pub struct ModeTreeCommandingHelper {
    pub current_mode: ModeAndSubmode,
    pub state: ModeTreeHelperState,
    pub children_mode_store: ModeStoreVec,
    pub target_tables: TargetModeTables,
    pub sequence_tables: SequenceModeTables,
    pub helper: SequenceExecutionHelper,
}

impl Default for ModeTreeCommandingHelper {
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

impl ModeTreeCommandingHelper {
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

    pub fn handle_mode_reply(&mut self, reply: &GenericMessage<ModeReply>) {
        let mut update_mode_store = |target_id, mode_and_submode| {
            if !self.children_mode_store.has_component(target_id) {
                return;
            }
            self.children_mode_store
                .set_mode_for_contained_component(target_id, mode_and_submode);
            match self.state {
                ModeTreeHelperState::Idle => (),
                ModeTreeHelperState::TargetKeeping => {}
                ModeTreeHelperState::SequenceCommanding => {
                    let mut still_awating_replies = false;
                    self.children_mode_store.0.iter_mut().for_each(|val| {
                        if val.id() == target_id {
                            val.awaiting_reply = false;
                        }
                        if val.awaiting_reply {
                            still_awating_replies = true;
                        }
                    });
                    if !still_awating_replies {
                        self.helper.confirm_sequence_done();
                    }
                }
            }
        };
        match reply.message {
            ModeReply::ModeInfo(mode_and_submode) => {
                update_mode_store(reply.sender_id(), mode_and_submode);
            }
            ModeReply::ModeReply(mode_and_submode) => {
                update_mode_store(reply.sender_id(), mode_and_submode);
            }
            ModeReply::CantReachMode(_) => (),
            ModeReply::WrongMode { reached, .. } => {
                update_mode_store(reply.sender_id(), reached);
            }
        };
    }

    pub fn start_command_sequence(
        &mut self,
        mode: Mode,
        request_id: RequestId,
    ) -> Result<(), ModeDoesNotExistError> {
        self.helper.load(mode, request_id, &self.sequence_tables)?;
        self.state = ModeTreeHelperState::SequenceCommanding;
        Ok(())
    }

    pub fn state_machine(
        &mut self,
        opt_reply: Option<GenericMessage<ModeReply>>,
        req_sender: &impl ModeRequestSender,
    ) -> Result<ModeTreeHelperResult, ModeTreeHelperError> {
        if let Some(reply) = opt_reply {
            self.handle_mode_reply(&reply);
        }
        match self.state {
            ModeTreeHelperState::Idle => Ok(ModeTreeHelperResult::Idle),
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
                Ok(ModeTreeHelperResult::TargetKeeping(TargetKeepingResult::Ok))
            }
            ModeTreeHelperState::SequenceCommanding => Ok(self
                .helper
                .run(
                    &self.sequence_tables,
                    req_sender,
                    &mut self.children_mode_store,
                )?
                .into()),
        }
    }
}

#[derive(Default, Debug)]
pub struct ModeRequestHandlerMock {
    get_mode_calls: RefCell<u32>,
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
pub struct ModeReplyHandlerMock {}

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
    pub mode_and_submode: ModeAndSubmode,
    pub target_mode_and_submode: Option<ModeAndSubmode>,
    pub subsystem_helper: ModeTreeCommandingHelper,
    pub mode_req_handler_mock: ModeRequestHandlerMock,
    pub mode_req_recvd: u32,
}

impl AcsSubsystem {
    pub fn new(mode_node: ModeRequestorAndHandlerMpscBounded) -> Self {
        Self {
            mode_node,
            mode_requestor_info: None,
            mode_and_submode: UNKNOWN_MODE,
            target_mode_and_submode: None,
            subsystem_helper: ModeTreeCommandingHelper::default(),
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
                ModeTreeHelperResult::Idle => todo!(),
                ModeTreeHelperResult::TargetKeeping(target_keeping_result) => todo!(),
                ModeTreeHelperResult::SequenceCommanding(sequence_handler_result) => todo!(),
            },
            Err(error) => match error {
                ModeTreeHelperError::Message(generic_targeted_messaging_error) => todo!(),
                ModeTreeHelperError::CurrentModeNotInTargetTable(_) => todo!(),
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
        self.mode_and_submode
    }
}

impl ModeRequestHandler for AcsSubsystem {
    type Error = ModeError;

    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
    ) -> Result<(), Self::Error> {
        self.mode_requestor_info = Some(requestor);
        self.target_mode_and_submode = Some(mode_and_submode);
        self.mode_req_handler_mock
            .start_transition(requestor, mode_and_submode, forced)
            .unwrap();
        // TODO: CHeck if a transition is already active. For now, we do not allow a new transition
        // if one is already active.
        // Execute mode map by executing the transition table(s).
        // TODO: How to deal with error handling? Add this error to generic ModeError, or create
        // new error type?
        self.subsystem_helper
            .start_command_sequence(mode_and_submode.mode(), requestor.request_id())
            .unwrap();
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
        self.mode_req_handler_mock
            .announce_mode(requestor_info, recursive);
    }

    fn handle_mode_reached(
        &mut self,
        requestor_info: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        if let Some(requestor) = requestor_info {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
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
        self.mode_node.send_mode_reply(requestor_info, reply)?;
        Ok(())
    }
}

struct MgmAssembly {
    pub mode_node: ModeRequestorAndHandlerMpscBounded,
    pub mode_requestor_info: Option<MessageMetadata>,
    pub mode_and_submode: ModeAndSubmode,
    pub target_mode_and_submode: Option<ModeAndSubmode>,
    pub mode_req_mock: ModeRequestHandlerMock,
    pub mode_msgs_recvd: u32,
}

impl MgmAssembly {
    pub fn new(mode_node: ModeRequestorAndHandlerMpscBounded) -> Self {
        Self {
            mode_node,
            mode_requestor_info: None,
            mode_and_submode: UNKNOWN_MODE,
            target_mode_and_submode: None,
            mode_req_mock: Default::default(),
            mode_msgs_recvd: 0,
        }
    }

    pub fn run(&mut self) {
        self.check_mode_requests().expect("mode messaging error");
        self.check_mode_replies().expect("mode messaging error");
    }
    pub fn get_and_clear_num_mode_msgs(&mut self) -> u32 {
        let tmp = self.mode_msgs_recvd;
        self.mode_msgs_recvd = 0;
        tmp
    }

    pub fn check_mode_requests(&mut self) -> Result<(), GenericTargetedMessagingError> {
        if let Some(request) = self.mode_node.try_recv_mode_request()? {
            self.mode_msgs_recvd += 1;
            self.handle_mode_request(request).unwrap();
        }
        Ok(())
    }

    pub fn check_mode_replies(&mut self) -> Result<(), GenericTargetedMessagingError> {
        // TODO: Call mode reply handler mock.
        if let Some(reply_and_id) = self.mode_node.try_recv_mode_reply()? {
            match reply_and_id.message {
                ModeReply::ModeReply(reply) => {
                    println!(
                        "TestAssembly: Received mode reply from {:?}, reached: {:?}",
                        reply_and_id.sender_id(),
                        reply
                    );
                }
                ModeReply::CantReachMode(_) => todo!(),
                ModeReply::WrongMode { expected, reached } => {
                    println!(
                        "TestAssembly: Wrong mode reply from {:?}, reached {:?}, expected {:?}",
                        reply_and_id.sender_id(),
                        reached,
                        expected
                    );
                }
                ModeReply::ModeInfo(_mode_and_submode) => {}
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
        self.mode_requestor_info = Some(requestor);
        self.target_mode_and_submode = Some(mode_and_submode);
        self.mode_req_mock
            .start_transition(requestor, mode_and_submode, forced)
            .unwrap();
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
        let mut acs_ctrl = AcsController {
            mode_node: acs_ctrl_node,
            mode_and_submode: UNKNOWN_MODE,
            announce_mode_queue: RefCell::new(Default::default()),
            mode_req_mock: Default::default(),
        };
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
            ModeAndSubmode::new(AcsMode::IDLE as Mode, 0),
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
}

#[test]
fn announce_recursively() {
    let mut tb = TreeTestbench::new();
    tb.pus.announce_modes_recursively();
    // Run everything twice so the order does not matter.
    for _ in 0..2 {
        tb.subsystem.run();
        tb.ctrl.run();
        tb.mgt_manager.run();
        tb.mgm_assy.run();
        tb.mgm_devs[0].run();
        tb.mgm_devs[1].run();
        tb.mgt_dev.run();
    }
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
    assert_eq!(tb.mgm_assy.get_and_clear_num_mode_msgs(), 1);
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
    tb.subsystem.run();
    tb.ctrl.run();
    tb.mgm_assy.run();
    tb.mgt_dev.run();
    tb.mgm_devs[0].run();
    tb.mgm_devs[1].run();
    tb.pus
        .send_mode_cmd(ModeAndSubmode::new(AcsMode::IDLE as u32, 0));
}
