use core::cell::Cell;
use std::{println, sync::mpsc};

use satrs::mode::{
    ModeError, ModeProvider, ModeReplyReceiver, ModeReplySender, ModeRequestHandler,
    ModeRequestHandlerMpscBounded, ModeRequestReceiver, ModeRequestSender,
    ModeRequestorAndHandlerMpscBounded, ModeRequestorBoundedMpsc,
};
use satrs::request::RequestId;
use satrs::{
    mode::{ModeAndSubmode, ModeReply, ModeRequest},
    queue::GenericTargetedMessagingError,
    request::GenericMessage,
    ChannelId,
};
use std::string::{String, ToString};

pub enum TestChannelId {
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
                TestChannelId::Assembly as u32,
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
    pub mode_requestor_info: Option<(RequestId, ChannelId)>,
}

impl TestDevice {
    pub fn run(&mut self) {
        self.check_mode_requests().expect("mode messaging error");
    }

    pub fn check_mode_requests(&mut self) -> Result<(), GenericTargetedMessagingError> {
        if let Some(request) = self.mode_node.try_recv_mode_request()? {
            match request.message {
                ModeRequest::SetMode(mode_and_submode) => {
                    self.start_transition(request.request_id, request.sender_id, mode_and_submode)
                        .unwrap();
                    self.mode_requestor_info = Some((request.request_id, request.sender_id));
                }
                ModeRequest::ReadMode => self
                    .mode_node
                    .send_mode_reply(
                        request.request_id,
                        request.sender_id,
                        ModeReply::ModeReply(self.mode_and_submode),
                    )
                    .unwrap(),
                ModeRequest::AnnounceMode => {
                    self.announce_mode(request.request_id, request.sender_id, false)
                }
                ModeRequest::AnnounceModeRecursive => {
                    self.announce_mode(request.request_id, request.sender_id, true)
                }
            }
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
    fn start_transition(
        &mut self,
        _request_id: RequestId,
        _sender_id: ChannelId,
        mode_and_submode: ModeAndSubmode,
    ) -> Result<(), ModeError> {
        self.mode_and_submode = mode_and_submode;
        self.handle_mode_reached()?;
        Ok(())
    }

    fn announce_mode(&self, _request_id: RequestId, _sender_id: ChannelId, _recursive: bool) {
        println!(
            "{}: announcing mode: {:?}",
            self.name, self.mode_and_submode
        );
    }

    fn handle_mode_reached(&mut self) -> Result<(), GenericTargetedMessagingError> {
        let (req_id, sender_id) = self.mode_requestor_info.unwrap();
        self.mode_node.send_mode_reply(
            req_id,
            sender_id,
            ModeReply::ModeReply(self.mode_and_submode),
        )?;
        Ok(())
    }
}

struct TestAssembly {
    pub mode_node: ModeRequestorAndHandlerMpscBounded,
    pub mode_requestor_info: Option<(RequestId, ChannelId)>,
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
                    self.start_transition(request.request_id, request.sender_id, mode_and_submode)
                        .unwrap();
                }
                ModeRequest::ReadMode => self
                    .mode_node
                    .send_mode_reply(
                        request.request_id,
                        request.sender_id,
                        ModeReply::ModeReply(self.mode_and_submode),
                    )
                    .unwrap(),
                ModeRequest::AnnounceMode => {
                    self.announce_mode(request.request_id, request.sender_id, false)
                }
                ModeRequest::AnnounceModeRecursive => {
                    self.announce_mode(request.request_id, request.sender_id, true)
                }
            }
        }
        Ok(())
    }

    pub fn check_mode_replies(&mut self) -> Result<(), GenericTargetedMessagingError> {
        if let Some(reply_and_id) = self.mode_node.try_recv_mode_reply()? {
            match reply_and_id.message {
                ModeReply::ModeInfo(_) => todo!(),
                ModeReply::ModeReply(reply) => {
                    println!(
                        "TestAssembly: Received mode reply from {:?}, reached: {:?}",
                        reply_and_id.sender_id, reply
                    );
                }
                ModeReply::CantReachMode(_) => todo!(),
                ModeReply::WrongMode { expected, reached } => {
                    println!(
                        "TestAssembly: Wrong mode reply from {:?}, reached {:?}, expected {:?}",
                        reply_and_id.sender_id, reached, expected
                    );
                }
            }
        }
        Ok(())
    }
}

impl ModeRequestHandler for TestAssembly {
    fn start_transition(
        &mut self,
        request_id: RequestId,
        sender_id: ChannelId,
        mode_and_submode: ModeAndSubmode,
    ) -> Result<(), ModeError> {
        self.mode_requestor_info = Some((request_id, sender_id));
        self.target_mode_and_submode = Some(mode_and_submode);
        Ok(())
    }

    fn announce_mode(&self, request_id: RequestId, _sender_id: ChannelId, recursive: bool) {
        println!(
            "TestAssembly: Announcing mode (recursively: {}): {:?}",
            recursive, self.mode_and_submode
        );
        // self.mode_requestor_info = Some((request_id, sender_id));
        let mut mode_request = ModeRequest::AnnounceMode;
        if recursive {
            mode_request = ModeRequest::AnnounceModeRecursive;
        }
        self.mode_node
            .request_sender_map
            .0
            .iter()
            .for_each(|(_, sender)| {
                sender
                    .send(GenericMessage::new(
                        request_id,
                        self.mode_node.local_channel_id_generic(),
                        mode_request,
                    ))
                    .expect("sending mode request failed");
            });
    }

    fn handle_mode_reached(&mut self) -> Result<(), GenericTargetedMessagingError> {
        let (req_id, sender_id) = self.mode_requestor_info.unwrap();
        self.mode_node.send_mode_reply(
            req_id,
            sender_id,
            ModeReply::ModeReply(self.mode_and_submode),
        )?;
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
        TestChannelId::Assembly as u32,
        request_receiver_assy,
        reply_receiver_assy,
    );
    // Mode requestors only.
    let mut mode_node_pus =
        ModeRequestorBoundedMpsc::new(TestChannelId::PusModeService as u32, reply_receiver_pus);

    // Request handlers only.
    let mut mode_node_dev1 =
        ModeRequestHandlerMpscBounded::new(TestChannelId::Device1 as u32, request_receiver_dev1);
    let mut mode_node_dev2 =
        ModeRequestHandlerMpscBounded::new(TestChannelId::Device2 as u32, request_receiver_dev2);

    // Set up mode request senders first.
    mode_node_pus.add_message_target(TestChannelId::Assembly as u32, request_sender_to_assy);
    mode_node_pus.add_message_target(
        TestChannelId::Device1 as u32,
        request_sender_to_dev1.clone(),
    );
    mode_node_pus.add_message_target(
        TestChannelId::Device2 as u32,
        request_sender_to_dev2.clone(),
    );
    mode_node_assy.add_request_target(TestChannelId::Device1 as u32, request_sender_to_dev1);
    mode_node_assy.add_request_target(TestChannelId::Device2 as u32, request_sender_to_dev2);

    // Set up mode reply senders.
    mode_node_dev1.add_message_target(TestChannelId::Assembly as u32, reply_sender_to_assy.clone());
    mode_node_dev1.add_message_target(
        TestChannelId::PusModeService as u32,
        reply_sender_to_pus.clone(),
    );
    mode_node_dev2.add_message_target(TestChannelId::Assembly as u32, reply_sender_to_assy);
    mode_node_dev2.add_message_target(
        TestChannelId::PusModeService as u32,
        reply_sender_to_pus.clone(),
    );
    mode_node_assy.add_reply_target(TestChannelId::PusModeService as u32, reply_sender_to_pus);

    let mut device1 = TestDevice {
        name: "Test Device 1".to_string(),
        mode_node: mode_node_dev1,
        mode_requestor_info: None,
        mode_and_submode: ModeAndSubmode::new(0, 0),
    };
    let mut device2 = TestDevice {
        name: "Test Device 2".to_string(),
        mode_node: mode_node_dev2,
        mode_requestor_info: None,
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
