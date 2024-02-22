use std::sync::mpsc;

use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::{
    mode::{Mode, ModeAndSubmode, ModeReply, ModeRequest, Submode},
    queue::GenericTargetedMessagingError,
    request::{
        MessageWithSenderId, MpscMessageReceiverWithIds, MpscMessageSenderAndReceiver,
        MpscMessageSenderMap, MpscMessageSenderWithId, MpscRequestAndReplySenderAndReceiver,
    },
    ChannelId,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TableEntryType {
    /// Target table containing information of the expected children modes for  given mode.
    Target,
    /// Sequence table which contains information about how to reach a target table, including
    /// the order of the sequences.
    Sequence,
}

pub struct ModeTableEntry {
    /// Name of respective table entry.
    pub name: &'static str,
    /// Target channel ID.
    pub channel_id: ChannelId,
    pub mode_submode: ModeAndSubmode,
    pub allowed_submode_mask: Option<Submode>,
    pub check_success: bool,
}

pub struct ModeTableMapValue {
    /// Name for a given mode table entry.
    pub name: &'static str,
    pub entries: Vec<ModeTableEntry>,
}

pub type ModeTable = HashMap<Mode, ModeTableMapValue>;

pub trait ModeRequestSender {
    fn local_channel_id(&self) -> ChannelId;
    fn send_mode_request(
        &self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError>;
}

pub trait ModeReplySender {
    fn local_channel_id(&self) -> ChannelId;

    fn send_mode_reply(
        &self,
        target_id: ChannelId,
        reply: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError>;
}

pub trait ModeRequestReceiver {
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeRequest>>, GenericTargetedMessagingError>;
}

pub trait ModeReplyReceiver {
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeReply>>, GenericTargetedMessagingError>;
}

impl MpscMessageSenderMap<ModeRequest> {
    pub fn send_mode_request(
        &self,
        local_id: ChannelId,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.send_message(local_id, target_id, request)
    }

    pub fn add_request_target(
        &mut self,
        target_id: ChannelId,
        request_sender: mpsc::Sender<MessageWithSenderId<ModeRequest>>,
    ) {
        self.add_message_target(target_id, request_sender)
    }
}

impl MpscMessageSenderMap<ModeReply> {
    pub fn send_mode_reply(
        &self,
        local_id: ChannelId,
        target_id: ChannelId,
        request: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.send_message(local_id, target_id, request)
    }

    pub fn add_reply_target(
        &mut self,
        target_id: ChannelId,
        request_sender: mpsc::Sender<MessageWithSenderId<ModeReply>>,
    ) {
        self.add_message_target(target_id, request_sender)
    }
}

impl ModeReplySender for MpscMessageSenderWithId<ModeReply> {
    fn send_mode_reply(
        &self,
        target_channel_id: ChannelId,
        reply: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.send_message(target_channel_id, reply)
    }

    fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id
    }
}

impl ModeRequestSender for MpscMessageSenderWithId<ModeRequest> {
    fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id
    }

    fn send_mode_request(
        &self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.send_message(target_id, request)
    }
}

impl ModeReplyReceiver for MpscMessageReceiverWithIds<ModeReply> {
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeReply>>, GenericTargetedMessagingError> {
        self.try_recv_message()
    }
}
impl ModeRequestReceiver for MpscMessageReceiverWithIds<ModeRequest> {
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeRequest>>, GenericTargetedMessagingError> {
        self.try_recv_message()
    }
}

impl<FROM> ModeRequestSender for MpscMessageSenderAndReceiver<ModeRequest, FROM> {
    fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id_generic()
    }

    fn send_mode_request(
        &self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.message_sender_map
            .send_mode_request(self.local_channel_id(), target_id, request)
    }
}

impl<FROM> ModeReplySender for MpscMessageSenderAndReceiver<ModeReply, FROM> {
    fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id_generic()
    }

    fn send_mode_reply(
        &self,
        target_id: ChannelId,
        request: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.message_sender_map
            .send_mode_reply(self.local_channel_id(), target_id, request)
    }
}

impl<TO> ModeReplyReceiver for MpscMessageSenderAndReceiver<TO, ModeReply> {
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeReply>>, GenericTargetedMessagingError> {
        self.message_receiver
            .try_recv_message(self.local_channel_id_generic())
    }
}
impl<TO> ModeRequestReceiver for MpscMessageSenderAndReceiver<TO, ModeRequest> {
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeRequest>>, GenericTargetedMessagingError> {
        self.message_receiver
            .try_recv_message(self.local_channel_id_generic())
    }
}

pub type MpscModeRequestHandlerConnector = MpscMessageSenderAndReceiver<ModeReply, ModeRequest>;
pub type MpscModeRequestorConnector = MpscMessageSenderAndReceiver<ModeRequest, ModeReply>;
pub type MpscModeConnector = MpscRequestAndReplySenderAndReceiver<ModeRequest, ModeReply>;

impl<REPLY> MpscRequestAndReplySenderAndReceiver<ModeRequest, REPLY> {
    pub fn add_request_target(
        &mut self,
        target_id: ChannelId,
        request_sender: mpsc::Sender<MessageWithSenderId<ModeRequest>>,
    ) {
        self.request_sender_map
            .add_message_target(target_id, request_sender)
    }
}

impl<REQUEST> MpscRequestAndReplySenderAndReceiver<REQUEST, ModeReply> {
    pub fn add_reply_target(
        &mut self,
        target_id: ChannelId,
        reply_sender: mpsc::Sender<MessageWithSenderId<ModeReply>>,
    ) {
        self.reply_sender_map
            .add_message_target(target_id, reply_sender)
    }
}

impl<REPLY> ModeRequestSender for MpscRequestAndReplySenderAndReceiver<ModeRequest, REPLY> {
    fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id_generic()
    }

    fn send_mode_request(
        &self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.request_sender_map
            .send_mode_request(self.local_channel_id(), target_id, request)
    }
}

impl<REQUEST> ModeReplySender for MpscRequestAndReplySenderAndReceiver<REQUEST, ModeReply> {
    fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id_generic()
    }

    fn send_mode_reply(
        &self,
        target_id: ChannelId,
        request: ModeReply,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.reply_sender_map
            .send_mode_reply(self.local_channel_id(), target_id, request)
    }
}

impl<REQUEST> ModeReplyReceiver for MpscRequestAndReplySenderAndReceiver<REQUEST, ModeReply> {
    fn try_recv_mode_reply(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeReply>>, GenericTargetedMessagingError> {
        self.reply_receiver
            .try_recv_message(self.local_channel_id_generic())
    }
}

impl<REPLY> ModeRequestReceiver for MpscRequestAndReplySenderAndReceiver<ModeRequest, REPLY> {
    fn try_recv_mode_request(
        &self,
    ) -> Result<Option<MessageWithSenderId<ModeRequest>>, GenericTargetedMessagingError> {
        self.request_receiver
            .try_recv_message(self.local_channel_id_generic())
    }
}

pub trait ModeProvider {
    fn mode_and_submode(&self) -> ModeAndSubmode;
}

#[derive(Debug, Clone)]
pub enum ModeError {
    Messaging(GenericTargetedMessagingError),
}

impl From<GenericTargetedMessagingError> for ModeError {
    fn from(value: GenericTargetedMessagingError) -> Self {
        Self::Messaging(value)
    }
}

pub trait ModeRequestHandler: ModeProvider {
    fn start_transition(&mut self, mode_and_submode: ModeAndSubmode) -> Result<(), ModeError>;

    fn announce_mode(&self, recursive: bool);
    fn handle_mode_reached(&mut self) -> Result<(), GenericTargetedMessagingError>;
}

#[cfg(test)]
mod tests {
    use std::{println, sync::mpsc};

    use alloc::string::{String, ToString};

    use super::*;
    use crate::{
        mode::{ModeAndSubmode, ModeReply, ModeRequest},
        mode_tree::ModeRequestSender,
        ChannelId,
    };

    pub enum TestChannelId {
        Device1 = 1,
        Device2 = 2,
        Assembly = 3,
        PusModeService = 4,
    }

    struct PusModeService {
        pub mode_node: MpscModeRequestorConnector,
    }

    impl PusModeService {
        pub fn send_announce_mode_cmd_to_assy(&self) {
            self.mode_node
                .send_mode_request(
                    TestChannelId::Assembly as u32,
                    ModeRequest::AnnounceModeRecursive,
                )
                .unwrap();
        }
    }

    struct TestDevice {
        pub name: String,
        pub mode_node: MpscModeRequestHandlerConnector,
        pub mode_and_submode: ModeAndSubmode,
        pub mode_req_commander: Option<ChannelId>,
    }

    impl TestDevice {
        pub fn run(&mut self) {
            self.check_mode_requests().expect("mode messaging error");
        }

        pub fn check_mode_requests(&mut self) -> Result<(), GenericTargetedMessagingError> {
            if let Some(request_and_id) = self.mode_node.try_recv_mode_request()? {
                match request_and_id.message {
                    ModeRequest::SetMode(mode_and_submode) => {
                        self.start_transition(mode_and_submode).unwrap();
                        self.mode_req_commander = Some(request_and_id.sender_id);
                    }
                    ModeRequest::ReadMode => self
                        .mode_node
                        .send_mode_reply(
                            request_and_id.sender_id,
                            ModeReply::ModeReply(self.mode_and_submode),
                        )
                        .unwrap(),
                    ModeRequest::AnnounceMode => self.announce_mode(false),
                    ModeRequest::AnnounceModeRecursive => self.announce_mode(true),
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
        fn start_transition(&mut self, mode_and_submode: ModeAndSubmode) -> Result<(), ModeError> {
            self.mode_and_submode = mode_and_submode;
            self.handle_mode_reached()?;
            Ok(())
        }

        fn announce_mode(&self, _recursive: bool) {
            println!(
                "{}: announcing mode: {:?}",
                self.name, self.mode_and_submode
            );
        }

        fn handle_mode_reached(&mut self) -> Result<(), GenericTargetedMessagingError> {
            self.mode_node.send_mode_reply(
                self.mode_req_commander.unwrap(),
                ModeReply::ModeReply(self.mode_and_submode),
            )?;
            Ok(())
        }
    }

    struct TestAssembly {
        pub mode_node: MpscModeConnector,
        pub mode_req_commander: Option<ChannelId>,
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
            if let Some(request_and_id) = self.mode_node.try_recv_mode_request()? {
                match request_and_id.message {
                    ModeRequest::SetMode(mode_and_submode) => {
                        self.start_transition(mode_and_submode).unwrap();
                        self.mode_req_commander = Some(request_and_id.sender_id);
                    }
                    ModeRequest::ReadMode => {
                        // self.handle_read_mode_request(0, self.mode_and_submode, &mut self.mode_reply_sender).unwrap()
                        self.mode_node
                            .send_mode_reply(
                                request_and_id.sender_id,
                                ModeReply::ModeReply(self.mode_and_submode),
                            )
                            .unwrap()
                    }
                    ModeRequest::AnnounceMode => self.announce_mode(false),
                    ModeRequest::AnnounceModeRecursive => self.announce_mode(true),
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
            mode_and_submode: ModeAndSubmode,
        ) -> Result<(), super::ModeError> {
            self.target_mode_and_submode = Some(mode_and_submode);
            Ok(())
        }

        fn announce_mode(&self, recursive: bool) {
            println!(
                "TestAssembly: Announcing mode (recursively: {}): {:?}",
                recursive, self.mode_and_submode
            );
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
                        .send(MessageWithSenderId::new(
                            self.mode_node.local_channel_id_generic(),
                            mode_request,
                        ))
                        .expect("sending mode request failed");
                });
        }

        fn handle_mode_reached(&mut self) -> Result<(), GenericTargetedMessagingError> {
            self.mode_node.send_mode_reply(
                self.mode_req_commander.unwrap(),
                ModeReply::ModeReply(self.mode_and_submode),
            )?;
            Ok(())
        }
    }

    #[test]
    fn basic_test() {
        // All request channel handles.
        let (request_sender_to_dev1, request_receiver_dev1) = mpsc::channel();
        let (request_sender_to_dev2, request_receiver_dev2) = mpsc::channel();
        let (request_sender_to_assy, request_receiver_assy) = mpsc::channel();

        // All reply channel handles.
        let (reply_sender_to_assy, reply_receiver_assy) = mpsc::channel();
        let (reply_sender_to_pus, reply_receiver_pus) = mpsc::channel();

        // Mode requestors and handlers.
        let mut mode_node_assy = MpscModeConnector::new(
            TestChannelId::Assembly as u32,
            request_receiver_assy,
            reply_receiver_assy,
        );
        // Mode requestors only.
        let mut mode_node_pus = MpscModeRequestorConnector::new(
            TestChannelId::PusModeService as u32,
            reply_receiver_pus,
        );

        // Request handlers only.
        let mut mode_node_dev1 = MpscModeRequestHandlerConnector::new(
            TestChannelId::Device1 as u32,
            request_receiver_dev1,
        );
        let mut mode_node_dev2 = MpscModeRequestHandlerConnector::new(
            TestChannelId::Device2 as u32,
            request_receiver_dev2,
        );

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
        mode_node_dev1
            .add_message_target(TestChannelId::Assembly as u32, reply_sender_to_assy.clone());
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
            mode_req_commander: None,
            mode_and_submode: ModeAndSubmode::new(0, 0),
        };
        let mut device2 = TestDevice {
            name: "Test Device 2".to_string(),
            mode_node: mode_node_dev2,
            mode_req_commander: None,
            mode_and_submode: ModeAndSubmode::new(0, 0),
        };
        let mut assy = TestAssembly {
            mode_node: mode_node_assy,
            mode_req_commander: None,
            mode_and_submode: ModeAndSubmode::new(0, 0),
            target_mode_and_submode: None,
        };
        let pus_service = PusModeService {
            mode_node: mode_node_pus,
        };

        pus_service.send_announce_mode_cmd_to_assy();
        assy.run();
        device1.run();
        device2.run();
        assy.run();
    }
}
