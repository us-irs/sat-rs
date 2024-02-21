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

pub struct StandardModeReplySender {
    pub local_id: ChannelId,
    pub reply_sender_map: HashMap<ChannelId, mpsc::Sender<ModeReply>>,
}

impl ModeReplySendProvider for StandardModeReplySender {
    fn send_mode_reply(
        &mut self,
        target_channel_id: ChannelId,
        reply: ModeReply,
    ) -> Result<(), ModeMessagingError> {
        if self.reply_sender_map.contains_key(&target_channel_id) {
            self.reply_sender_map
                .get(&target_channel_id)
                .unwrap()
                .send(reply)
                .map_err(|_| GenericSendError::RxDisconnected)?;
            return Ok(());
        }
        Err(ModeMessagingError::TargetDoesNotExist(target_channel_id))
    }

    fn local_channel_id(&self) -> ChannelId {
        self.local_id
    }
}

impl StandardModeReplySender {
    pub fn new(local_id: ChannelId) -> Self {
        Self {
            local_id,
            reply_sender_map: HashMap::new(),
        }
    }

    pub fn add_reply_target(
        &mut self,
        target_id: ChannelId,
        reply_sender: mpsc::Sender<ModeReply>,
    ) {
        self.reply_sender_map.insert(target_id, reply_sender);
    }
}

pub struct ModeReplyTuple {
    pub sender_id: ChannelId,
    pub reply: ModeReply,
}

pub trait ModeReplyReceiver {
    fn try_recv_mode_reply(&self) -> Result<Option<ModeReplyTuple>, ModeMessagingError>;
}

pub struct StandardModeReplyReceiver {
    pub sender_id: ChannelId,
    pub receiver: mpsc::Receiver<ModeReply>,
}

#[derive(Default)]
pub struct StandardModeReplyReceiverList {
    pub reply_receiver_vec: Vec<StandardModeReplyReceiver>,
}

impl StandardModeReplyReceiverList {
    pub fn add_reply_receiver(
        &mut self,
        sender_id: ChannelId,
        receiver: mpsc::Receiver<ModeReply>,
    ) {
        self.reply_receiver_vec.push(StandardModeReplyReceiver {
            sender_id,
            receiver,
        });
    }
}

impl ModeReplyReceiver for StandardModeReplyReceiverList {
    fn try_recv_mode_reply(&self) -> Result<Option<ModeReplyTuple>, ModeMessagingError> {
        for reply_receiver in &self.reply_receiver_vec {
            match reply_receiver.receiver.try_recv() {
                Ok(reply) => {
                    return Ok(Some(ModeReplyTuple {
                        sender_id: reply_receiver.sender_id,
                        reply,
                    }))
                }
                Err(e) => {
                    if e == mpsc::TryRecvError::Disconnected {
                        return Err(GenericReceiveError::TxDisconnected(Some(
                            reply_receiver.sender_id,
                        ))
                        .into());
                    }
                }
            }
        }
        Ok(None)
    }
}

pub trait ModeRequestSendProvider {
    fn local_channel_id(&self) -> Option<ChannelId>;
    fn send_mode_request(
        &mut self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), ModeMessagingError>;
}

pub struct StandardModeRequestSender {
    pub local_channel_id: ChannelId,
    pub request_sender_map: HashMap<ChannelId, mpsc::Sender<ModeRequestWithSenderId>>,
}

impl ModeRequestSendProvider for StandardModeRequestSender {
    fn send_mode_request(
        &mut self,
        target_id: ChannelId,
        request: ModeRequest,
    ) -> Result<(), ModeMessagingError> {
        if self.request_sender_map.contains_key(&target_id) {
            self.request_sender_map
                .get(&target_id)
                .unwrap()
                .send(ModeRequestWithSenderId {
                    sender_id: target_id,
                    request,
                })
                .map_err(|_| GenericSendError::RxDisconnected)?;
            return Ok(());
        }
        Err(ModeMessagingError::TargetDoesNotExist(target_id))
    }

    fn local_channel_id(&self) -> Option<ChannelId> {
        Some(self.local_channel_id)
    }
}

impl StandardModeRequestSender {
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

    use hashbrown::HashMap;

    use crate::{
        mode::{ModeAndSubmode, ModeReply, ModeRequest},
        mode_tree::ModeRequestSendProvider,
        ChannelId,
    };

    use super::{
        ModeMessagingError, ModeProvider, ModeReplySendProvider, ModeRequestHandler,
        ModeRequestWithSenderId, StandardModeReplyReceiverList, StandardModeReplySender,
        StandardModeRequestSender,
    };

    struct TestDevice {
        /// One receiver handle for all mode requests.
        pub mode_req_receiver: mpsc::Receiver<ModeRequestWithSenderId>,
        /// This structure contains all handles to send mode replies.
        pub mode_reply_sender: StandardModeReplySender,
        pub last_mode_sender: Option<ChannelId>,
        pub mode_and_submode: ModeAndSubmode,
        pub target_mode_and_submode: Option<ModeAndSubmode>,
    }

    struct TestAssembly {
        /// One receiver handle for all mode requests.
        pub mode_req_receiver: mpsc::Receiver<ModeRequestWithSenderId>,
        /// This structure contains all handles to send mode replies.
        pub mode_reply_sender: StandardModeReplySender,
        /// This structure contains all handles to send mode requests to its children.
        pub mode_children_map: StandardModeRequestSender,
        pub mode_reply_receiver_list: StandardModeReplyReceiverList,
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
            self.check_mode_requests()
        }

        pub fn check_mode_requests(&mut self) {
            match self.mode_req_receiver.try_recv() {
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
                "Announcing mode (recursively: {}): {:?}",
                recursive, self.mode_and_submode
            );
            let mut mode_request = ModeRequest::AnnounceMode;
            if recursive {
                mode_request = ModeRequest::AnnounceModeRecursive;
            }
            self.mode_children_map
                .request_sender_map
                .iter()
                .for_each(|(_, sender)| {
                    sender
                        .send(ModeRequestWithSenderId::new(
                            self.mode_children_map.local_channel_id().unwrap(),
                            mode_request,
                        ))
                        .expect("sending mode request failed");
                });
        }

        fn handle_mode_reached(&mut self) -> Result<(), ModeMessagingError> {
            self.mode_reply_sender.send_mode_reply(
                self.last_mode_sender.unwrap(),
                ModeReply::ModeReply(self.mode_and_submode),
            )?;
            Ok(())
        }
    }

    pub enum TestChannelId {
        Device1 = 1,
        Device2 = 2,
        Assembly = 3,
    }
    #[test]
    fn basic_test() {
        let (dev1_sender, dev1_receiver) = mpsc::channel();
        let (dev2_sender, dev2_receiver) = mpsc::channel();
        let mut mode_reply_sender_dev1 =
            StandardModeReplySender::new(TestChannelId::Device1 as u32);
        let mut mode_reply_sender_dev2 =
            StandardModeReplySender::new(TestChannelId::Device2 as u32);
        let mut mode_request_map_assy =
            StandardModeRequestSender::new(TestChannelId::Assembly as u32);
        let mut mode_reply_map_assy = StandardModeReplySender::new(TestChannelId::Assembly as u32);

        let (dev1_reply_sender, assy_reply_receiver_dev1) = mpsc::channel();
        let (dev2_reply_sender, assy_reply_receiver_dev2) = mpsc::channel();
        mode_reply_sender_dev1.add_reply_target(TestChannelId::Assembly as u32, dev1_reply_sender);
        mode_reply_sender_dev2.add_reply_target(TestChannelId::Assembly as u32, dev2_reply_sender);
        //mode_reply_map_assy.add_reply_target(target_id, reply_sender)

        let device1 = TestDevice {
            mode_req_receiver: dev1_receiver,
            mode_reply_sender: mode_reply_sender_dev1,
            last_mode_sender: None,
            mode_and_submode: ModeAndSubmode::new(0, 0),
            target_mode_and_submode: None,
        };
        let device2 = TestDevice {
            mode_req_receiver: dev2_receiver,
            mode_reply_sender: mode_reply_sender_dev2,
            last_mode_sender: None,
            mode_and_submode: ModeAndSubmode::new(0, 0),
            target_mode_and_submode: None,
        };
    }
}
