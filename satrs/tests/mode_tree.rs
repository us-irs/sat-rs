use core::cell::Cell;
use std::{println, sync::mpsc};

use satrs::mode::{
    Mode, ModeError, ModeProvider, ModeReplyReceiver, ModeReplySender, ModeRequestHandler,
    ModeRequestHandlerMpscBounded, ModeRequestReceiver, ModeRequestSender,
    ModeRequestorAndHandlerMpscBounded, ModeRequestorBoundedMpsc,
};
use satrs::mode_tree::alloc_mod::{
    ModeStoreVec, SequenceModeTables, SequenceTableMapValue, TargetModeTables, TargetTableMapValue,
};
use satrs::mode_tree::ModeStoreProvider;
use satrs::request::{MessageMetadata, MessageSender};
use satrs::{
    mode::{ModeAndSubmode, ModeReply, ModeRequest},
    queue::GenericTargetedMessagingError,
    request::GenericMessage,
    ComponentId,
};
use std::string::{String, ToString};

const INVALID_MODE: ModeAndSubmode = ModeAndSubmode::new(0xffffffff, 0);
const UNKNOWN_MODE: ModeAndSubmode = ModeAndSubmode::new(0xffffffff - 1, 0);

pub enum TestComponentId {
    MagnetometerDevice = 1,
    MagnetorquerDevice = 2,
    ReactionWheelDevice = 3,
    StartrackerDevice = 4,
    ReactionWheelAssembly = 8,
    MagnetometerAssembly = 9,
    AcsController = 14,
    AcsSubsystem = 15,
    PusModeService = 16,
}

pub type RequestSenderType = mpsc::SyncSender<GenericMessage<ModeRequest>>;
pub type ReplySenderType = mpsc::SyncSender<GenericMessage<ModeReply>>;

/// Trait which denotes that an object is a parent in a mode tree.
///
/// A mode parent is capable of sending mode requests to child objects and has a unique component
/// ID.
pub trait ModeParent {
    type Sender: MessageSender<ModeRequest>;

    fn id(&self) -> ComponentId;
    fn add_mode_child(&mut self, id: ComponentId, request_sender: Self::Sender);
}

/// Trait which denotes that an object is a child in a mode tree.
///
/// A child is capable of sending mode replies to parent objects and has a unique component ID.
pub trait ModeChild {
    type Sender: MessageSender<ModeReply>;

    fn id(&self) -> ComponentId;
    fn add_mode_parent(&mut self, id: ComponentId, reply_sender: Self::Sender);
}

/// Utility method which connects a mode tree parent object to a child object by calling
/// [ModeParent::add_mode_child] on the [parent][ModeParent] and calling
/// [ModeChild::add_mode_parent] on the [child][ModeChild].
///
/// # Arguments
///
/// * `parent` - The parent object which implements [ModeParent].
/// * `request_sender` - Sender object to send mode requests to the child.
/// * `child` - The child object which implements [ModeChild].
/// * `reply_sender` - Sender object to send mode replies to the parent.
pub fn connect_mode_nodes<ReqSender, ReplySender>(
    parent: &mut impl ModeParent<Sender=ReqSender>,
    request_sender: ReqSender,
    child: &mut impl ModeChild<Sender=ReplySender>,
    reply_sender: ReplySender,
) {
    parent.add_mode_child(child.id(), request_sender);
    child.add_mode_parent(parent.id(), reply_sender);
}

struct PusModeService {
    pub request_id_counter: Cell<u32>,
    pub mode_node: ModeRequestorBoundedMpsc,
}

impl PusModeService {
    pub fn send_announce_mode_cmd_to_subsystem(&self) {
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
}

impl ModeParent for PusModeService {
    type Sender = RequestSenderType;

    fn id(&self) -> ComponentId {
        TestComponentId::PusModeService as ComponentId
    }

    fn add_mode_child(&mut self, id: ComponentId, request_sender: RequestSenderType) {
        self.mode_node.add_message_target(id, request_sender);
    }
}

struct AcsSubsystem {
    pub mode_node: ModeRequestorAndHandlerMpscBounded,
    pub mode_requestor_info: Option<MessageMetadata>,
    pub mode_and_submode: ModeAndSubmode,
    pub target_mode_and_submode: Option<ModeAndSubmode>,
    pub children_mode_store: ModeStoreVec,
    pub target_tables: TargetModeTables,
    pub sequence_tables: SequenceModeTables,
}

impl AcsSubsystem {
    pub fn add_target_and_sequence_table(
        &mut self,
        mode: Mode,
        target_table_val: TargetTableMapValue,
        sequence_table_val: SequenceTableMapValue,
    ) {
        self.target_tables.0.insert(mode, target_table_val);
        self.sequence_tables.0.insert(mode, sequence_table_val);
    }
}

impl ModeParent for AcsSubsystem {
    type Sender = RequestSenderType;

    fn id(&self) -> ComponentId {
        TestComponentId::PusModeService as ComponentId
    }

    fn add_mode_child(&mut self, id: ComponentId, request_sender: RequestSenderType) {
        self.children_mode_store.add_component(id, UNKNOWN_MODE);
        self.mode_node.add_request_target(id, request_sender);
    }
}

impl ModeChild for AcsSubsystem {
    type Sender = ReplySenderType;

    fn id(&self) -> ComponentId {
        TestComponentId::PusModeService as ComponentId
    }
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
    ) -> Result<(), Self::Error> {
        self.mode_requestor_info = Some(requestor);
        self.target_mode_and_submode = Some(mode_and_submode);
        // Execute mode map by executing the transition table(s).
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
            .request_sender_map
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
    }

    fn handle_mode_reached(
        &mut self,
        requestor_info: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        if let Some(requestor) = requestor_info {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
        }
        Ok(())
    }

    fn handle_mode_info(
        &mut self,
        requestor_info: MessageMetadata,
        info: ModeAndSubmode,
    ) -> Result<(), Self::Error> {
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
        self.mode_node.send_mode_reply(requestor_info, reply)?;
        Ok(())
    }
}

struct MgmAssembly {
    pub mode_node: ModeRequestorAndHandlerMpscBounded,
    pub mode_requestor_info: Option<MessageMetadata>,
    pub mode_and_submode: ModeAndSubmode,
    pub target_mode_and_submode: Option<ModeAndSubmode>,
}

impl MgmAssembly {
    pub fn run(&mut self) {
        self.check_mode_requests().expect("mode messaging error");
        self.check_mode_replies().expect("mode messaging error");
    }

    pub fn check_mode_requests(&mut self) -> Result<(), GenericTargetedMessagingError> {
        if let Some(request) = self.mode_node.try_recv_mode_request()? {
            match request.message {
                ModeRequest::SetMode(mode_and_submode) => {
                    self.start_transition(request.requestor_info, mode_and_submode)
                        .unwrap();
                }
                ModeRequest::ReadMode => self
                    .mode_node
                    .send_mode_reply(
                        request.requestor_info,
                        ModeReply::ModeReply(self.mode_and_submode),
                    )
                    .unwrap(),
                ModeRequest::AnnounceMode => {
                    self.announce_mode(Some(request.requestor_info), false)
                }
                ModeRequest::AnnounceModeRecursive => {
                    self.announce_mode(Some(request.requestor_info), true)
                }
                ModeRequest::ModeInfo(_) => todo!(),
            }
        }
        Ok(())
    }

    pub fn check_mode_replies(&mut self) -> Result<(), GenericTargetedMessagingError> {
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
            }
        }
        Ok(())
    }
}

impl ModeParent for MgmAssembly {
    type Sender = RequestSenderType;

    fn id(&self) -> ComponentId {
        TestComponentId::AcsSubsystem as ComponentId
    }

    fn add_mode_child(&mut self, id: ComponentId, request_sender: RequestSenderType) {
        self.mode_node.add_request_target(id, request_sender);
    }
}

impl ModeChild for MgmAssembly {
    type Sender = ReplySenderType;

    fn id(&self) -> ComponentId {
        TestComponentId::PusModeService as ComponentId
    }

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
    ) -> Result<(), Self::Error> {
        self.mode_requestor_info = Some(requestor);
        self.target_mode_and_submode = Some(mode_and_submode);
        Ok(())
    }

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool) {
        println!(
            "TestAssembly: Announcing mode (recursively: {}): {:?}",
            recursive, self.mode_and_submode
        );
        // self.mode_requestor_info = Some((request_id, sender_id));
        let mut mode_request = ModeRequest::AnnounceMode;
        if recursive {
            mode_request = ModeRequest::AnnounceModeRecursive;
        }
        let request_id = requestor_info.map_or(0, |info| info.request_id());
        self.mode_node
            .request_sender_map
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
    }

    fn handle_mode_reached(
        &mut self,
        mode_requestor: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        if let Some(requestor) = mode_requestor {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
        }
        Ok(())
    }

    fn send_mode_reply(
        &self,
        requestor: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), Self::Error> {
        self.mode_node.send_mode_reply(requestor, reply)?;
        Ok(())
    }

    fn handle_mode_info(
        &mut self,
        _requestor_info: MessageMetadata,
        _info: ModeAndSubmode,
    ) -> Result<(), Self::Error> {
        // TODO: A proper assembly must reach to mode changes of its children..
        Ok(())
    }
}

struct CommonDevice {
    name: String,
    pub id: ComponentId,
    pub mode_node: ModeRequestHandlerMpscBounded,
    pub mode_and_submode: ModeAndSubmode,
}

impl CommonDevice {
    pub fn new(name: String, id: ComponentId, mode_node: ModeRequestHandlerMpscBounded) -> Self {
        Self {
            name,
            id,
            mode_node,
            mode_and_submode: ModeAndSubmode::new(0, 0),
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

impl ModeChild for CommonDevice {
    type Sender = ReplySenderType;

    fn id(&self) -> ComponentId {
        self.id
    }

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
    ) -> Result<(), ModeError> {
        self.mode_and_submode = mode_and_submode;
        self.handle_mode_reached(Some(requestor))?;
        Ok(())
    }

    fn announce_mode(&self, _requestor_info: Option<MessageMetadata>, _recursive: bool) {
        println!(
            "{}: announcing mode: {:?}",
            self.name, self.mode_and_submode
        );
    }

    fn handle_mode_reached(&mut self, requestor: Option<MessageMetadata>) -> Result<(), ModeError> {
        if let Some(requestor) = requestor {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
        }
        Ok(())
    }
    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), ModeError> {
        self.mode_node.send_mode_reply(requestor_info, reply)?;
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
        Ok(())
    }
}

pub struct AcsController {
    pub mode_node: ModeRequestHandlerMpscBounded,
    pub mode_and_submode: ModeAndSubmode,
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

impl ModeChild for AcsController {
    type Sender = ReplySenderType;

    fn id(&self) -> ComponentId {
        TestComponentId::AcsController as u64
    }

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
    ) -> Result<(), Self::Error> {
        self.mode_and_submode = mode_and_submode;
        self.handle_mode_reached(Some(requestor))?;
        Ok(())
    }

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool) {
        println!(
            "ACS Controllerj: announcing mode: {:?}",
            self.mode_and_submode
        );
    }

    fn handle_mode_reached(
        &mut self,
        requestor_info: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        if let Some(requestor) = requestor_info {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
        }
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
        Ok(())
    }

    fn send_mode_reply(
        &self,
        requestor_info: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), Self::Error> {
        self.mode_node.send_mode_reply(requestor_info, reply)?;
        Ok(())
    }
}

#[test]
fn main() {
    // All request channel handles.
    let (request_sender_to_mgm_dev, request_receiver_mgm_dev) = mpsc::sync_channel(10);
    let (request_sender_to_mgt_dev, request_receiver_mgt_dev) = mpsc::sync_channel(10);
    let (request_sender_to_mgm_assy, request_receiver_mgm_assy) = mpsc::sync_channel(10);
    let (request_sender_to_acs_subsystem, request_receiver_acs_subsystem) = mpsc::sync_channel(10);
    let (request_sender_to_acs_ctrl, request_receiver_acs_ctrl) = mpsc::sync_channel(10);

    // All reply channel handles.
    let (reply_sender_to_mgm_assy, reply_receiver_mgm_assy) = mpsc::sync_channel(10);
    let (reply_sender_to_acs_subsystem, reply_receiver_acs_subsystem) = mpsc::sync_channel(10);
    let (reply_sender_to_pus, reply_receiver_pus) = mpsc::sync_channel(10);

    // Mode requestors and handlers.
    let mut mgm_assy_node = ModeRequestorAndHandlerMpscBounded::new(
        TestComponentId::MagnetometerAssembly as ComponentId,
        request_receiver_mgm_assy,
        reply_receiver_mgm_assy,
    );
    let mut acs_subsystem_node = ModeRequestorAndHandlerMpscBounded::new(
        TestComponentId::AcsSubsystem as ComponentId,
        request_receiver_acs_subsystem,
        reply_receiver_acs_subsystem,
    );
    // Mode requestors only.
    let mut mode_node_pus = ModeRequestorBoundedMpsc::new(
        TestComponentId::PusModeService as ComponentId,
        reply_receiver_pus,
    );

    // Request handlers only.
    let mut mgm_dev_node = ModeRequestHandlerMpscBounded::new(
        TestComponentId::MagnetometerDevice as ComponentId,
        request_receiver_mgm_dev,
    );
    let mut mgt_dev_node = ModeRequestHandlerMpscBounded::new(
        TestComponentId::MagnetorquerDevice as ComponentId,
        request_receiver_mgt_dev,
    );
    let mut acs_ctrl_node = ModeRequestHandlerMpscBounded::new(
        TestComponentId::AcsController as ComponentId,
        request_receiver_acs_ctrl,
    );

    // Set up mode reply senders.
    mgm_dev_node.add_message_target(
        TestComponentId::MagnetometerAssembly as ComponentId,
        reply_sender_to_mgm_assy.clone(),
    );
    mgt_dev_node.add_message_target(
        TestComponentId::MagnetometerAssembly as ComponentId,
        reply_sender_to_mgm_assy.clone(),
    );
    mgm_assy_node.add_reply_target(
        TestComponentId::PusModeService as ComponentId,
        reply_sender_to_pus.clone(),
    );
    mgm_assy_node.add_reply_target(
        TestComponentId::AcsSubsystem as ComponentId,
        reply_sender_to_pus.clone(),
    );

    let mut mode_store_acs_subsystem = ModeStoreVec::default();
    let mut target_tables_acs_subsystem = TargetModeTables::default();
    let mut sequence_tables_acs_subsystem = SequenceModeTables::default();

    let mut mgm_dev = CommonDevice::new(
        "MGM".to_string(),
        TestComponentId::MagnetometerDevice as u64,
        mgm_dev_node,
    );
    let mut mgt_dev = CommonDevice::new(
        "MGT".to_string(),
        TestComponentId::MagnetorquerDevice as u64,
        mgt_dev_node,
    );
    let mut mgm_assy = MgmAssembly {
        mode_node: mgm_assy_node,
        mode_requestor_info: None,
        mode_and_submode: ModeAndSubmode::new(0, 0),
        target_mode_and_submode: None,
    };
    let mut acs_subsystem = AcsSubsystem {
        mode_node: acs_subsystem_node,
        mode_requestor_info: None,
        mode_and_submode: ModeAndSubmode::new(0, 0),
        target_mode_and_submode: None,
        children_mode_store: mode_store_acs_subsystem,
        target_tables: target_tables_acs_subsystem,
        sequence_tables: sequence_tables_acs_subsystem,
    };
    let mut acs_ctrl = AcsController {
        mode_node: acs_ctrl_node,
        mode_and_submode: ModeAndSubmode::new(0, 0),
    };
    let mut pus_service = PusModeService {
        request_id_counter: Cell::new(0),
        mode_node: mode_node_pus,
    };

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
        request_sender_to_mgm_dev.clone(),
        &mut mgm_dev,
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
        request_sender_to_mgt_dev,
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
        &mut mgm_assy,
        request_sender_to_mgm_dev,
        &mut mgm_dev,
        reply_sender_to_mgm_assy,
    );

    pus_service.send_announce_mode_cmd_to_subsystem();
    mgm_assy.run();
    mgm_dev.run();
    mgt_dev.run();
}
