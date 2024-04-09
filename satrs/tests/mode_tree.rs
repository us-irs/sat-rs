use core::cell::Cell;
use std::{println, sync::mpsc};

use satrs::mode::{
    ModeError, ModeProvider, ModeReplyReceiver, ModeReplySender, ModeRequestHandler,
    ModeRequestHandlerMpscBounded, ModeRequestReceiver, ModeRequestorAndHandlerMpscBounded,
    ModeRequestorBoundedMpsc,
};
use satrs::request::MessageMetadata;
use satrs::{
    mode::{ModeAndSubmode, ModeReply, ModeRequest},
    queue::GenericTargetedMessagingError,
    request::GenericMessage,
    ComponentId,
};
use std::string::{String, ToString};

pub enum TestComponentId {
    Device1 = 1,
    Device2 = 2,
    Assembly = 3,
    PusModeService = 4,
}

struct PusModeService {
    pub request_id_counter: Cell<u32>,
    pub mode_node: ModeRequestorBoundedMpsc,
}

impl PusModeService {
    pub fn send_announce_mode_cmd_to_assy(&self) {
        self.mode_node
            .send_mode_request(
                self.request_id_counter.get(),
                TestComponentId::Assembly as ComponentId,
                ModeRequest::AnnounceModeRecursive,
            )
            .unwrap();
        self.request_id_counter
            .replace(self.request_id_counter.get() + 1);
    }
}

struct TestDevice {
    pub name: String,
    pub mode_node: ModeRequestHandlerMpscBounded,
    pub mode_and_submode: ModeAndSubmode,
}

impl TestDevice {
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

impl ModeProvider for TestDevice {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl ModeRequestHandler for TestDevice {
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

struct TestAssembly {
    pub mode_node: ModeRequestorAndHandlerMpscBounded,
    pub mode_requestor_info: Option<MessageMetadata>,
    pub mode_and_submode: ModeAndSubmode,
    pub target_mode_and_submode: Option<ModeAndSubmode>,
}

impl ModeProvider for TestAssembly {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl TestAssembly {
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

impl ModeRequestHandler for TestAssembly {
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

fn main() {
    // All request channel handles.
    let (request_sender_to_dev1, request_receiver_dev1) = mpsc::sync_channel(10);
    let (request_sender_to_dev2, request_receiver_dev2) = mpsc::sync_channel(10);
    let (request_sender_to_assy, request_receiver_assy) = mpsc::sync_channel(10);

    // All reply channel handles.
    let (reply_sender_to_assy, reply_receiver_assy) = mpsc::sync_channel(10);
    let (reply_sender_to_pus, reply_receiver_pus) = mpsc::sync_channel(10);

    // Mode requestors and handlers.
    let mut mode_node_assy = ModeRequestorAndHandlerMpscBounded::new(
        TestComponentId::Assembly as ComponentId,
        request_receiver_assy,
        reply_receiver_assy,
    );
    // Mode requestors only.
    let mut mode_node_pus = ModeRequestorBoundedMpsc::new(
        TestComponentId::PusModeService as ComponentId,
        reply_receiver_pus,
    );

    // Request handlers only.
    let mut mode_node_dev1 = ModeRequestHandlerMpscBounded::new(
        TestComponentId::Device1 as ComponentId,
        request_receiver_dev1,
    );
    let mut mode_node_dev2 = ModeRequestHandlerMpscBounded::new(
        TestComponentId::Device2 as ComponentId,
        request_receiver_dev2,
    );

    // Set up mode request senders first.
    mode_node_pus.add_message_target(
        TestComponentId::Assembly as ComponentId,
        request_sender_to_assy,
    );
    mode_node_pus.add_message_target(
        TestComponentId::Device1 as ComponentId,
        request_sender_to_dev1.clone(),
    );
    mode_node_pus.add_message_target(
        TestComponentId::Device2 as ComponentId,
        request_sender_to_dev2.clone(),
    );
    mode_node_assy.add_request_target(
        TestComponentId::Device1 as ComponentId,
        request_sender_to_dev1,
    );
    mode_node_assy.add_request_target(
        TestComponentId::Device2 as ComponentId,
        request_sender_to_dev2,
    );

    // Set up mode reply senders.
    mode_node_dev1.add_message_target(
        TestComponentId::Assembly as ComponentId,
        reply_sender_to_assy.clone(),
    );
    mode_node_dev1.add_message_target(
        TestComponentId::PusModeService as ComponentId,
        reply_sender_to_pus.clone(),
    );
    mode_node_dev2.add_message_target(
        TestComponentId::Assembly as ComponentId,
        reply_sender_to_assy,
    );
    mode_node_dev2.add_message_target(
        TestComponentId::PusModeService as ComponentId,
        reply_sender_to_pus.clone(),
    );
    mode_node_assy.add_reply_target(
        TestComponentId::PusModeService as ComponentId,
        reply_sender_to_pus,
    );

    let mut device1 = TestDevice {
        name: "Test Device 1".to_string(),
        mode_node: mode_node_dev1,
        mode_and_submode: ModeAndSubmode::new(0, 0),
    };
    let mut device2 = TestDevice {
        name: "Test Device 2".to_string(),
        mode_node: mode_node_dev2,
        mode_and_submode: ModeAndSubmode::new(0, 0),
    };
    let mut assy = TestAssembly {
        mode_node: mode_node_assy,
        mode_requestor_info: None,
        mode_and_submode: ModeAndSubmode::new(0, 0),
        target_mode_and_submode: None,
    };
    let pus_service = PusModeService {
        request_id_counter: Cell::new(0),
        mode_node: mode_node_pus,
    };

    pus_service.send_announce_mode_cmd_to_assy();
    assy.run();
    device1.run();
    device2.run();
    assy.run();
}
