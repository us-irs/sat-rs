use std::sync::mpsc;

use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::{
    mode::{Mode, ModeAndSubmode, ModeReply, ModeRequest, Submode},
    queue::{GenericReceiveError, GenericSendError},
    ChannelId,
};

pub struct ModeRequestWithSenderId {
    pub sender_id: ChannelId,
    pub request: ModeRequest,
}

impl ModeRequestWithSenderId {
    pub fn new(sender_id: ChannelId, request: ModeRequest) -> Self {
        Self { sender_id, request }
    }
}

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

#[derive(Debug, Clone)]
pub enum ModeMessagingError {
    TargetDoesNotExist(ChannelId),
    Send(GenericSendError),
    Receive(GenericReceiveError),
}
impl From<GenericSendError> for ModeMessagingError {
    fn from(value: GenericSendError) -> Self {
        Self::Send(value)
    }
}

impl From<GenericReceiveError> for ModeMessagingError {
    fn from(value: GenericReceiveError) -> Self {
        Self::Receive(value)
    }
}

pub trait ModeReplySendProvider {
    fn local_channel_id(&self) -> ChannelId;

    fn send_mode_reply(
        &mut self,
        target_id: ChannelId,
        reply: ModeReply,
    ) -> Result<(), ModeMessagingError>;
}

pub struct MpscModeReplyRouter {
    pub local_id: ChannelId,
    pub reply_sender_map: HashMap<ChannelId, mpsc::Sender<ModeReplyWithSenderId>>,
}

impl ModeReplySendProvider for MpscModeReplyRouter {
    fn send_mode_reply(
        &mut self,
        target_channel_id: ChannelId,
        reply: ModeReply,
    ) -> Result<(), ModeMessagingError> {
        if self.reply_sender_map.contains_key(&target_channel_id) {
            self.reply_sender_map
                .get(&target_channel_id)
                .unwrap()
                .send(ModeReplyWithSenderId {
                    sender_id: self.local_channel_id(),
                    reply,
                })
                .map_err(|_| GenericSendError::RxDisconnected)?;
            return Ok(());
        }
        Err(ModeMessagingError::TargetDoesNotExist(target_channel_id))
    }

    fn local_channel_id(&self) -> ChannelId {
        self.local_id
    }
}

impl MpscModeReplyRouter {
    pub fn new(local_id: ChannelId) -> Self {
        Self {
            local_id,
            reply_sender_map: HashMap::new(),
        }
    }

    pub fn add_reply_target(
        &mut self,
        target_id: ChannelId,
        reply_sender: mpsc::Sender<ModeReplyWithSenderId>,
    ) {
        self.reply_sender_map.insert(target_id, reply_sender);
    }
}

pub struct ModeReplyWithSenderId {
    pub sender_id: ChannelId,
    pub reply: ModeReply,
}

pub trait ModeReplyReceiver {
    fn try_recv_mode_reply(&self) -> Result<Option<ModeReplyWithSenderId>, ModeMessagingError>;
}

pub struct MpscModeReplyReceiver {
    local_channel_id: ChannelId,
    reply_receiver: mpsc::Receiver<ModeReplyWithSenderId>,
}

impl MpscModeReplyReceiver {
    pub fn new(
        local_channel_id: ChannelId,
        reply_receiver: mpsc::Receiver<ModeReplyWithSenderId>,
    ) -> Self {
        Self {
            local_channel_id,
            reply_receiver,
        }
    }

    pub fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id
    }
}

impl ModeReplyReceiver for MpscModeReplyReceiver {
    fn try_recv_mode_reply(&self) -> Result<Option<ModeReplyWithSenderId>, ModeMessagingError> {
        match self.reply_receiver.try_recv() {
            Ok(reply) => {
                return Ok(Some(reply));
            }
            Err(e) => {
                if e == mpsc::TryRecvError::Disconnected {
                    return Err(
                        GenericReceiveError::TxDisconnected(Some(self.local_channel_id())).into(),
                    );
                }
            }
        }
        Ok(None)
    }
}

pub trait ModeRequestReceiver {
    fn try_recv_mode_reply(&self) -> Result<Option<ModeRequestWithSenderId>, ModeMessagingError>;
}

pub struct MpscModeRequestReceiver {
    local_channel_id: ChannelId,
    receiver: mpsc::Receiver<ModeRequestWithSenderId>,
}

impl MpscModeRequestReceiver {
    pub fn new(
        local_channel_id: ChannelId,
        receiver: mpsc::Receiver<ModeRequestWithSenderId>,
    ) -> Self {
        Self {
            local_channel_id,
            receiver,
        }
    }

    pub fn local_channel_id(&self) -> ChannelId {
        self.local_channel_id
    }
}

impl ModeRequestReceiver for MpscModeRequestReceiver {
    fn try_recv_mode_reply(&self) -> Result<Option<ModeRequestWithSenderId>, ModeMessagingError> {
        match self.receiver.try_recv() {
            Ok(request_and_sender) => {
                return Ok(Some(ModeRequestWithSenderId {
                    sender_id: request_and_sender.sender_id,
                    request: request_and_sender.request,
                }))
            }
            Err(e) => {
                if e == mpsc::TryRecvError::Disconnected {
                    return Err(
                        GenericReceiveError::TxDisconnected(Some(self.local_channel_id)).into(),
                    );
                }
            }
        }
        Ok(None)
    }
}

pub trait ModeRequestSendProvider {
    fn local_channel_id(&self) -> Option<ChannelId>;
    fn send_mode_request(
        &self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), ModeMessagingError>;
}

pub struct MpscModeRequestRouter {
    pub local_channel_id: ChannelId,
    pub request_sender_map: HashMap<ChannelId, mpsc::Sender<ModeRequestWithSenderId>>,
}

impl ModeRequestSendProvider for MpscModeRequestRouter {
    fn send_mode_request(
        &self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), ModeMessagingError> {
        if self.request_sender_map.contains_key(&target_id) {
            self.request_sender_map
                .get(&target_id)
                .unwrap()
                .send(ModeRequestWithSenderId::new(
                    self.local_channel_id().unwrap(),
                    request,
                ))
                .map_err(|_| GenericSendError::RxDisconnected)?;
            return Ok(());
        }
        Err(ModeMessagingError::TargetDoesNotExist(target_id))
    }

    fn local_channel_id(&self) -> Option<ChannelId> {
        Some(self.local_channel_id)
    }
}

impl MpscModeRequestRouter {
    pub fn new(local_channel_id: ChannelId) -> Self {
        Self {
            local_channel_id,
            request_sender_map: HashMap::new(),
        }
    }

    pub fn add_request_target(
        &mut self,
        target_id: ChannelId,
        request_sender: mpsc::Sender<ModeRequestWithSenderId>,
    ) {
        self.request_sender_map.insert(target_id, request_sender);
    }
}

pub trait ModeProvider {
    fn mode_and_submode(&self) -> ModeAndSubmode;
}

#[derive(Debug, Clone)]
pub enum ModeError {
    Messaging(ModeMessagingError),
}

pub trait ModeRequestHandler: ModeProvider {
    fn start_transition(&mut self, mode_and_submode: ModeAndSubmode) -> Result<(), ModeError>;

    fn announce_mode(&self, recursive: bool);
    fn handle_mode_reached(&mut self) -> Result<(), ModeMessagingError>;
}

#[cfg(test)]
mod tests {
    use std::{println, sync::mpsc};

    use super::*;
    use crate::{
        mode::{ModeAndSubmode, ModeReply, ModeRequest},
        mode_tree::ModeRequestSendProvider,
        ChannelId,
    };

    pub enum TestChannelId {
        Device1 = 1,
        Device2 = 2,
        Assembly = 3,
        PusModeService = 4,
    }

    struct PusModeService {
        // This contains ALL objects which are able to process mode requests.
        pub mode_request_recipients: MpscModeRequestRouter,
        // This contains all reply receivers for request recipients.
        pub mode_reply_receiver_list: MpscModeReplyReceiver,
    }

    impl PusModeService {
        pub fn send_announce_mode_cmd_to_assy(&self) {
            self.mode_request_recipients
                .send_mode_request(
                    TestChannelId::Assembly as u32,
                    ModeRequest::AnnounceModeRecursive,
                )
                .unwrap();
        }
    }

    struct TestDevice {
        // This object is used to receive all mode requests, for example by the PUS Mode service
        // or by the parent assembly.
        pub mode_request_receiver: MpscModeRequestReceiver,
        // This structure contains all handles to send mode replies back to the senders.
        pub mode_reply_router: MpscModeReplyRouter,
        // Used to cache the receiver for mode replies.
        pub mode_reply_receiver: Option<ChannelId>,
        pub mode_and_submode: ModeAndSubmode,
        pub target_mode_and_submode: Option<ModeAndSubmode>,
    }

    struct TestAssembly {
        // This object is used to receive all mode requests, for example by the PUS Mode service
        // or by the parent subsystem.
        pub mode_request_receiver: MpscModeRequestReceiver,
        pub mode_reply_receiver: MpscModeReplyReceiver,
        // This structure contains all handles to send mode replies.
        pub mode_reply_senders: MpscModeReplyRouter,
        // This structure contains all handles to send mode requests to its children.
        pub mode_request_senders: MpscModeRequestRouter,
        pub last_mode_sender: Option<ChannelId>,
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
            // self.check_mode_requests()
        }

        /*
        pub fn check_mode_requests(&mut self) -> Result<(), ModeMessagingError> {
            match self.mode_request_receiver_list.try_recv_mode_reply()? {
                Some(request_and_id) => {
                    match request_and_id {

                    }

                }
                Ok(ModeRequestWithSenderId { sender_id, request }) => {
                    match request {
                        ModeRequest::SetMode(mode_and_submode) => {
                            self.start_transition(mode_and_submode).unwrap();
                            self.last_mode_sender = Some(sender_id);
                        }
                        ModeRequest::ReadMode => {
                            // self.handle_read_mode_request(0, self.mode_and_submode, &mut self.mode_reply_sender).unwrap()
                            self.mode_reply_sender
                                .send_mode_reply(
                                    sender_id,
                                    ModeReply::ModeReply(self.mode_and_submode),
                                )
                                .unwrap()
                        }
                        ModeRequest::AnnounceMode => self.announce_mode(false),
                        ModeRequest::AnnounceModeRecursive => self.announce_mode(true),
                    }
                }
                Err(_) => todo!(),
            };
        } */
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
                "Announcing mode (recursively: {}): {:?}",
                recursive, self.mode_and_submode
            );
            let mut mode_request = ModeRequest::AnnounceMode;
            if recursive {
                mode_request = ModeRequest::AnnounceModeRecursive;
            }
            self.mode_request_senders
                .request_sender_map
                .iter()
                .for_each(|(_, sender)| {
                    sender
                        .send(ModeRequestWithSenderId::new(
                            self.mode_request_senders.local_channel_id().unwrap(),
                            mode_request,
                        ))
                        .expect("sending mode request failed");
                });
        }

        fn handle_mode_reached(&mut self) -> Result<(), ModeMessagingError> {
            self.mode_reply_senders.send_mode_reply(
                self.last_mode_sender.unwrap(),
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

        // All request routers.
        let mut mode_request_router_assy =
            MpscModeRequestRouter::new(TestChannelId::Assembly as u32);
        let mut mode_request_router_pus_mode =
            MpscModeRequestRouter::new(TestChannelId::PusModeService as u32);

        // All request receivers.
        let mode_request_receiver_dev1 =
            MpscModeRequestReceiver::new(TestChannelId::Device1 as u32, request_receiver_dev1);
        let mode_request_receiver_dev2 =
            MpscModeRequestReceiver::new(TestChannelId::Device2 as u32, request_receiver_dev2);
        let mode_request_receiver_assy =
            MpscModeRequestReceiver::new(TestChannelId::Assembly as u32, request_receiver_assy);

        // All reply senders.
        let mut mode_reply_senders_dev1 = MpscModeReplyRouter::new(TestChannelId::Device1 as u32);
        let mut mode_reply_senders_dev2 = MpscModeReplyRouter::new(TestChannelId::Device2 as u32);
        let mut mode_reply_senders_assy = MpscModeReplyRouter::new(TestChannelId::Assembly as u32);

        // All reply receivers.
        let mode_reply_receiver_list_assy =
            MpscModeReplyReceiver::new(TestChannelId::Assembly as u32, reply_receiver_assy);
        let mode_reply_receiver_pus_service =
            MpscModeReplyReceiver::new(TestChannelId::PusModeService as u32, reply_receiver_pus);

        // Set up mode request senders first.
        mode_request_router_pus_mode
            .add_request_target(TestChannelId::Assembly as u32, request_sender_to_assy);
        mode_request_router_pus_mode.add_request_target(
            TestChannelId::Device1 as u32,
            request_sender_to_dev1.clone(),
        );
        mode_request_router_pus_mode.add_request_target(
            TestChannelId::Device2 as u32,
            request_sender_to_dev2.clone(),
        );
        mode_request_router_assy
            .add_request_target(TestChannelId::Device1 as u32, request_sender_to_dev1);
        mode_request_router_assy
            .add_request_target(TestChannelId::Device2 as u32, request_sender_to_dev2);

        // Set up mode reply senders.
        mode_reply_senders_dev1
            .add_reply_target(TestChannelId::Assembly as u32, reply_sender_to_assy.clone());
        mode_reply_senders_dev1.add_reply_target(
            TestChannelId::PusModeService as u32,
            reply_sender_to_pus.clone(),
        );
        mode_reply_senders_dev2
            .add_reply_target(TestChannelId::Assembly as u32, reply_sender_to_assy);
        mode_reply_senders_dev2.add_reply_target(
            TestChannelId::PusModeService as u32,
            reply_sender_to_pus.clone(),
        );
        mode_reply_senders_assy
            .add_reply_target(TestChannelId::PusModeService as u32, reply_sender_to_pus);

        let device1 = TestDevice {
            mode_request_receiver: mode_request_receiver_dev1,
            mode_reply_router: mode_reply_senders_dev1,
            mode_reply_receiver: None,
            mode_and_submode: ModeAndSubmode::new(0, 0),
            target_mode_and_submode: None,
        };
        let device2 = TestDevice {
            mode_request_receiver: mode_request_receiver_dev2,
            mode_reply_router: mode_reply_senders_dev2,
            mode_reply_receiver: None,
            mode_and_submode: ModeAndSubmode::new(0, 0),
            target_mode_and_submode: None,
        };
        let assy = TestAssembly {
            mode_request_receiver: mode_request_receiver_assy,
            mode_reply_senders: mode_reply_senders_assy,
            mode_request_senders: mode_request_router_assy,
            mode_reply_receiver: mode_reply_receiver_list_assy,
            last_mode_sender: None,
            mode_and_submode: ModeAndSubmode::new(0, 0),
            target_mode_and_submode: None,
        };
        let pus_service = PusModeService {
            mode_request_recipients: mode_request_router_pus_mode,
            mode_reply_receiver_list: mode_reply_receiver_pus_service,
        };

        pus_service.send_announce_mode_cmd_to_assy();
        // assy.run();
        // device1.run();
        // device2.run();
    }
}
