use std::{
    cell::RefCell,
    collections::VecDeque,
    sync::{mpsc, Arc, Mutex},
};

use derive_new::new;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use satrs::{
    hk::{HkRequest, HkRequestVariant},
    mode::{
        ModeAndSubmode, ModeError, ModeProvider, ModeReply, ModeRequestHandler,
        ModeRequestHandlerMpscBounded,
    },
    mode_tree::{ModeChild, ModeNode},
    power::SwitchRequest,
    pus::{EcssTmSender, PusTmVariant},
    queue::GenericSendError,
    request::{GenericMessage, MessageMetadata, UniqueApidTargetId},
    spacepackets::ByteConversionError,
};
use satrs_example::{
    config::{
        components::{NO_SENDER, PCDU_HANDLER},
        pus::PUS_MODE_SERVICE,
    },
    DeviceMode, TimestampHelper,
};
use satrs_minisim::{
    eps::{
        PcduReply, PcduRequest, PcduSwitch, SwitchMap, SwitchMapBinaryWrapper, SwitchMapWrapper,
    },
    SerializableSimMsgPayload, SimReply, SimRequest,
};
use serde::{Deserialize, Serialize};

use crate::{
    hk::PusHkHelper,
    pus::hk::{HkReply, HkReplyVariant},
    requests::CompositeRequest,
    tmtc::sender::TmTcSender,
};

pub trait SerialInterface {
    type Error: core::fmt::Debug;

    /// Send some data via the serial interface.
    fn send(&self, data: &[u8]) -> Result<(), Self::Error>;
    /// Receive all replies received on the serial interface so far. This function takes a closure
    /// and call its for each received packet, passing the received packet into it.
    fn try_recv_replies<ReplyHandler: FnMut(&[u8])>(
        &self,
        f: ReplyHandler,
    ) -> Result<(), Self::Error>;
}

#[derive(new)]
pub struct SerialInterfaceToSim {
    pub sim_request_tx: mpsc::Sender<SimRequest>,
    pub sim_reply_rx: mpsc::Receiver<SimReply>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[repr(u32)]
pub enum SetId {
    SwitcherSet = 0,
}

impl SerialInterface for SerialInterfaceToSim {
    type Error = ();

    fn send(&self, data: &[u8]) -> Result<(), Self::Error> {
        let request: PcduRequest = serde_json::from_slice(data).expect("expected a PCDU request");
        self.sim_request_tx
            .send(SimRequest::new_with_epoch_time(request))
            .expect("failed to send request to simulation");
        Ok(())
    }

    fn try_recv_replies<ReplyHandler: FnMut(&[u8])>(
        &self,
        mut f: ReplyHandler,
    ) -> Result<(), Self::Error> {
        loop {
            match self.sim_reply_rx.try_recv() {
                Ok(reply) => {
                    let reply = serde_json::to_string(&reply).unwrap();
                    f(reply.as_bytes());
                }
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => break,
                    mpsc::TryRecvError::Disconnected => {
                        log::warn!("sim reply sender has disconnected");
                        break;
                    }
                },
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct SerialInterfaceDummy {
    // Need interior mutability here for both fields.
    pub switch_map: RefCell<SwitchMapBinaryWrapper>,
    pub reply_deque: RefCell<VecDeque<SimReply>>,
}

impl SerialInterface for SerialInterfaceDummy {
    type Error = ();

    fn send(&self, data: &[u8]) -> Result<(), Self::Error> {
        let pcdu_req: PcduRequest = serde_json::from_slice(data).unwrap();
        let switch_map_mut = &mut self.switch_map.borrow_mut().0;
        match pcdu_req {
            PcduRequest::SwitchDevice { switch, state } => {
                match switch_map_mut.entry(switch) {
                    std::collections::hash_map::Entry::Occupied(mut val) => {
                        *val.get_mut() = state;
                    }
                    std::collections::hash_map::Entry::Vacant(vacant) => {
                        vacant.insert(state);
                    }
                };
            }
            PcduRequest::RequestSwitchInfo => {
                let mut reply_deque_mut = self.reply_deque.borrow_mut();
                reply_deque_mut.push_back(SimReply::new(&PcduReply::SwitchInfo(
                    switch_map_mut.clone(),
                )));
            }
        };
        Ok(())
    }

    fn try_recv_replies<ReplyHandler: FnMut(&[u8])>(
        &self,
        mut f: ReplyHandler,
    ) -> Result<(), Self::Error> {
        if self.reply_queue_empty() {
            return Ok(());
        }
        loop {
            let reply = self.get_next_reply_as_string();
            f(reply.as_bytes());
            if self.reply_queue_empty() {
                break;
            }
        }
        Ok(())
    }
}

impl SerialInterfaceDummy {
    fn get_next_reply_as_string(&self) -> String {
        let mut reply_deque_mut = self.reply_deque.borrow_mut();
        let next_reply = reply_deque_mut.pop_front().unwrap();
        serde_json::to_string(&next_reply).unwrap()
    }

    fn reply_queue_empty(&self) -> bool {
        self.reply_deque.borrow().is_empty()
    }
}

pub enum SerialSimInterfaceWrapper {
    Dummy(SerialInterfaceDummy),
    Sim(SerialInterfaceToSim),
}

impl SerialInterface for SerialSimInterfaceWrapper {
    type Error = ();

    fn send(&self, data: &[u8]) -> Result<(), Self::Error> {
        match self {
            SerialSimInterfaceWrapper::Dummy(dummy) => dummy.send(data),
            SerialSimInterfaceWrapper::Sim(sim) => sim.send(data),
        }
    }

    fn try_recv_replies<ReplyHandler: FnMut(&[u8])>(
        &self,
        f: ReplyHandler,
    ) -> Result<(), Self::Error> {
        match self {
            SerialSimInterfaceWrapper::Dummy(dummy) => dummy.try_recv_replies(f),
            SerialSimInterfaceWrapper::Sim(sim) => sim.try_recv_replies(f),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum OpCode {
    RegularOp = 0,
    PollAndRecvReplies = 1,
}

#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SwitchSet {
    pub valid: bool,
    pub switch_map: SwitchMap,
}

pub type SharedSwitchSet = Arc<Mutex<SwitchSet>>;

/// Example PCDU device handler.
#[derive(new)]
#[allow(clippy::too_many_arguments)]
pub struct PcduHandler<ComInterface: SerialInterface> {
    id: UniqueApidTargetId,
    dev_str: &'static str,
    mode_node: ModeRequestHandlerMpscBounded,
    composite_request_rx: mpsc::Receiver<GenericMessage<CompositeRequest>>,
    hk_reply_tx: mpsc::SyncSender<GenericMessage<HkReply>>,
    switch_request_rx: mpsc::Receiver<GenericMessage<SwitchRequest>>,
    tm_sender: TmTcSender,
    pub com_interface: ComInterface,
    shared_switch_map: Arc<Mutex<SwitchSet>>,
    #[new(value = "PusHkHelper::new(id)")]
    hk_helper: PusHkHelper,
    #[new(value = "ModeAndSubmode::new(satrs_example::DeviceMode::Off as u32, 0)")]
    mode_and_submode: ModeAndSubmode,
    #[new(default)]
    stamp_helper: TimestampHelper,
    #[new(value = "[0; 256]")]
    tm_buf: [u8; 256],
}

impl<ComInterface: SerialInterface> PcduHandler<ComInterface> {
    pub fn periodic_operation(&mut self, op_code: OpCode) {
        match op_code {
            OpCode::RegularOp => {
                self.stamp_helper.update_from_now();
                // Handle requests.
                self.handle_composite_requests();
                self.handle_mode_requests();
                self.handle_switch_requests();
                // Poll the switch states and/or telemetry regularly here.
                if self.mode() == DeviceMode::Normal as u32 || self.mode() == DeviceMode::On as u32
                {
                    self.handle_periodic_commands();
                }
            }
            OpCode::PollAndRecvReplies => {
                self.poll_and_handle_replies();
            }
        }
    }

    pub fn handle_composite_requests(&mut self) {
        loop {
            match self.composite_request_rx.try_recv() {
                Ok(ref msg) => match &msg.message {
                    CompositeRequest::Hk(hk_request) => {
                        self.handle_hk_request(&msg.requestor_info, hk_request)
                    }
                    // TODO: This object does not have actions (yet).. Still send back completion failure
                    // reply.
                    CompositeRequest::Action(_action_req) => {}
                },

                Err(e) => {
                    if e != mpsc::TryRecvError::Empty {
                        log::warn!(
                            "{}: failed to receive composite request: {:?}",
                            self.dev_str,
                            e
                        );
                    } else {
                        break;
                    }
                }
            }
        }
    }

    pub fn handle_hk_request(&mut self, requestor_info: &MessageMetadata, hk_request: &HkRequest) {
        match hk_request.variant {
            HkRequestVariant::OneShot => {
                if hk_request.unique_id == SetId::SwitcherSet as u32 {
                    if let Ok(hk_tm) = self.hk_helper.generate_hk_report_packet(
                        self.stamp_helper.stamp(),
                        SetId::SwitcherSet as u32,
                        &mut |hk_buf| {
                            // Send TM down as JSON.
                            let switch_map_snapshot = self
                                .shared_switch_map
                                .lock()
                                .expect("failed to lock switch map")
                                .clone();
                            let switch_map_json = serde_json::to_string(&switch_map_snapshot)
                                .expect("failed to serialize switch map");
                            if switch_map_json.len() > hk_buf.len() {
                                log::error!("switch map JSON too large for HK buffer");
                                return Err(ByteConversionError::ToSliceTooSmall {
                                    found: hk_buf.len(),
                                    expected: switch_map_json.len(),
                                });
                            }
                            Ok(switch_map_json.len())
                        },
                        &mut self.tm_buf,
                    ) {
                        self.tm_sender
                            .send_tm(self.id.id(), PusTmVariant::Direct(hk_tm))
                            .expect("failed to send HK TM");
                        self.hk_reply_tx
                            .send(GenericMessage::new(
                                *requestor_info,
                                HkReply::new(hk_request.unique_id, HkReplyVariant::Ack),
                            ))
                            .expect("failed to send HK reply");
                    }
                }
            }
            HkRequestVariant::EnablePeriodic => todo!(),
            HkRequestVariant::DisablePeriodic => todo!(),
            HkRequestVariant::ModifyCollectionInterval(_) => todo!(),
        }
    }

    pub fn handle_periodic_commands(&self) {
        let pcdu_req = PcduRequest::RequestSwitchInfo;
        let pcdu_req_ser = serde_json::to_string(&pcdu_req).unwrap();
        if let Err(_e) = self.com_interface.send(pcdu_req_ser.as_bytes()) {
            log::warn!("polling PCDU switch info failed");
        }
    }

    pub fn handle_mode_requests(&mut self) {
        loop {
            // TODO: Only allow one set mode request per cycle?
            match self.mode_node.try_recv_mode_request() {
                Ok(opt_msg) => {
                    if let Some(msg) = opt_msg {
                        let result = self.handle_mode_request(msg);
                        // TODO: Trigger event?
                        if result.is_err() {
                            log::warn!(
                                "{}: mode request failed with error {:?}",
                                self.dev_str,
                                result.err().unwrap()
                            );
                        }
                    } else {
                        break;
                    }
                }
                Err(e) => match e {
                    satrs::queue::GenericReceiveError::Empty => {
                        break;
                    }
                    satrs::queue::GenericReceiveError::TxDisconnected(_) => {
                        log::warn!("{}: failed to receive mode request: {:?}", self.dev_str, e);
                    }
                },
            }
        }
    }

    pub fn handle_switch_requests(&mut self) {
        loop {
            match self.switch_request_rx.try_recv() {
                Ok(switch_req) => match PcduSwitch::try_from(switch_req.message.switch_id()) {
                    Ok(pcdu_switch) => {
                        let pcdu_req = PcduRequest::SwitchDevice {
                            switch: pcdu_switch,
                            state: switch_req.message.target_state(),
                        };
                        let pcdu_req_ser = serde_json::to_string(&pcdu_req).unwrap();
                        self.com_interface
                            .send(pcdu_req_ser.as_bytes())
                            .expect("failed to send switch request to PCDU");
                    }
                    Err(e) => todo!("failed to convert switch ID {:?} to typed PCDU switch", e),
                },
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => break,
                    mpsc::TryRecvError::Disconnected => {
                        log::warn!("switch request receiver has disconnected");
                        break;
                    }
                },
            };
        }
    }

    pub fn poll_and_handle_replies(&mut self) {
        if let Err(e) = self.com_interface.try_recv_replies(|reply| {
            let sim_reply: SimReply = serde_json::from_slice(reply).expect("invalid reply format");
            let pcdu_reply = PcduReply::from_sim_message(&sim_reply).expect("invalid reply format");
            match pcdu_reply {
                PcduReply::SwitchInfo(switch_info) => {
                    let switch_map_wrapper =
                        SwitchMapWrapper::from_binary_switch_map_ref(&switch_info);
                    let mut shared_switch_map = self
                        .shared_switch_map
                        .lock()
                        .expect("failed to lock switch map");
                    shared_switch_map.switch_map = switch_map_wrapper.0;
                    shared_switch_map.valid = true;
                }
            }
        }) {
            log::warn!("receiving PCDU replies failed: {:?}", e);
        }
    }
}

impl<ComInterface: SerialInterface> ModeProvider for PcduHandler<ComInterface> {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl<ComInterface: SerialInterface> ModeRequestHandler for PcduHandler<ComInterface> {
    type Error = ModeError;
    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        _forced: bool,
    ) -> Result<(), satrs::mode::ModeError> {
        log::info!(
            "{}: transitioning to mode {:?}",
            self.dev_str,
            mode_and_submode
        );
        self.mode_and_submode = mode_and_submode;
        if mode_and_submode.mode() == DeviceMode::Off as u32 {
            self.shared_switch_map.lock().unwrap().valid = false;
        }
        self.handle_mode_reached(Some(requestor))?;
        Ok(())
    }

    fn announce_mode(&self, _requestor_info: Option<MessageMetadata>, _recursive: bool) {
        log::info!(
            "{} announcing mode: {:?}",
            self.dev_str,
            self.mode_and_submode
        );
    }

    fn handle_mode_reached(
        &mut self,
        requestor: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        self.announce_mode(requestor, false);
        if let Some(requestor) = requestor {
            if requestor.sender_id() == NO_SENDER {
                return Ok(());
            }
            if requestor.sender_id() != PUS_MODE_SERVICE.id() {
                log::warn!(
                    "can not send back mode reply to sender {}",
                    requestor.sender_id()
                );
            } else {
                self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode()))?;
            }
        }
        Ok(())
    }

    fn send_mode_reply(
        &self,
        requestor: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), Self::Error> {
        if requestor.sender_id() != PUS_MODE_SERVICE.id() {
            log::warn!(
                "can not send back mode reply to sender {}",
                requestor.sender_id()
            );
        }
        self.mode_node
            .send_mode_reply(requestor, reply)
            .map_err(|_| GenericSendError::RxDisconnected)?;
        Ok(())
    }

    fn handle_mode_info(
        &mut self,
        _requestor_info: MessageMetadata,
        _info: ModeAndSubmode,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<ComInterface: SerialInterface> ModeNode for PcduHandler<ComInterface> {
    fn id(&self) -> satrs::ComponentId {
        PCDU_HANDLER.into()
    }
}

impl<ComInterface: SerialInterface> ModeChild for PcduHandler<ComInterface> {
    type Sender = mpsc::SyncSender<GenericMessage<ModeReply>>;

    fn add_mode_parent(&mut self, id: satrs::ComponentId, reply_sender: Self::Sender) {
        self.mode_node.add_message_target(id, reply_sender);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use satrs::{
        mode::ModeRequest, power::SwitchStateBinary, request::GenericMessage, tmtc::PacketAsVec,
    };
    use satrs_example::config::{
        acs::MGM_HANDLER_0,
        components::{Apid, EPS_SUBSYSTEM, PCDU_HANDLER},
    };
    use satrs_minisim::eps::SwitchMapBinary;

    use super::*;

    #[derive(Default)]
    pub struct SerialInterfaceTest {
        pub inner: SerialInterfaceDummy,
        pub send_queue: RefCell<VecDeque<Vec<u8>>>,
        pub reply_queue: RefCell<VecDeque<String>>,
    }

    impl SerialInterface for SerialInterfaceTest {
        type Error = ();

        fn send(&self, data: &[u8]) -> Result<(), Self::Error> {
            let mut send_queue_mut = self.send_queue.borrow_mut();
            send_queue_mut.push_back(data.to_vec());
            self.inner.send(data)
        }

        fn try_recv_replies<ReplyHandler: FnMut(&[u8])>(
            &self,
            mut f: ReplyHandler,
        ) -> Result<(), Self::Error> {
            if self.inner.reply_queue_empty() {
                return Ok(());
            }
            loop {
                let reply = self.inner.get_next_reply_as_string();
                self.reply_queue.borrow_mut().push_back(reply.clone());
                f(reply.as_bytes());
                if self.inner.reply_queue_empty() {
                    break;
                }
            }
            Ok(())
        }
    }

    pub struct PcduTestbench {
        pub mode_request_tx: mpsc::SyncSender<GenericMessage<ModeRequest>>,
        pub mode_reply_rx_to_pus: mpsc::Receiver<GenericMessage<ModeReply>>,
        pub mode_reply_rx_to_parent: mpsc::Receiver<GenericMessage<ModeReply>>,
        pub composite_request_tx: mpsc::Sender<GenericMessage<CompositeRequest>>,
        pub hk_reply_rx: mpsc::Receiver<GenericMessage<HkReply>>,
        pub tm_rx: mpsc::Receiver<PacketAsVec>,
        pub switch_request_tx: mpsc::Sender<GenericMessage<SwitchRequest>>,
        pub handler: PcduHandler<SerialInterfaceTest>,
    }

    impl PcduTestbench {
        pub fn new() -> Self {
            let (mode_request_tx, mode_request_rx) = mpsc::sync_channel(5);
            let (mode_reply_tx_to_pus, mode_reply_rx_to_pus) = mpsc::sync_channel(5);
            let (mode_reply_tx_to_parent, mode_reply_rx_to_parent) = mpsc::sync_channel(5);
            let mode_node =
                ModeRequestHandlerMpscBounded::new(PCDU_HANDLER.into(), mode_request_rx);
            let (composite_request_tx, composite_request_rx) = mpsc::channel();
            let (hk_reply_tx, hk_reply_rx) = mpsc::sync_channel(10);
            let (tm_tx, tm_rx) = mpsc::sync_channel::<PacketAsVec>(5);
            let (switch_request_tx, switch_reqest_rx) = mpsc::channel();
            let shared_switch_map = Arc::new(Mutex::new(SwitchSet::default()));
            let mut handler = PcduHandler::new(
                UniqueApidTargetId::new(Apid::Eps as u16, 0),
                "TEST_PCDU",
                mode_node,
                composite_request_rx,
                hk_reply_tx,
                switch_reqest_rx,
                TmTcSender::Heap(tm_tx.clone()),
                SerialInterfaceTest::default(),
                shared_switch_map,
            );
            handler.add_mode_parent(EPS_SUBSYSTEM.into(), mode_reply_tx_to_parent);
            handler.add_mode_parent(PUS_MODE_SERVICE.into(), mode_reply_tx_to_pus);
            Self {
                mode_request_tx,
                mode_reply_rx_to_pus,
                mode_reply_rx_to_parent,
                composite_request_tx,
                hk_reply_rx,
                tm_rx,
                switch_request_tx,
                handler,
            }
        }

        pub fn verify_switch_info_req_was_sent(&self, expected_queue_len: usize) {
            // Check that there is now communication happening.
            let mut send_queue_mut = self.handler.com_interface.send_queue.borrow_mut();
            assert_eq!(send_queue_mut.len(), expected_queue_len);
            let packet_sent = send_queue_mut.pop_front().unwrap();
            drop(send_queue_mut);
            let pcdu_req: PcduRequest = serde_json::from_slice(&packet_sent).unwrap();
            assert_eq!(pcdu_req, PcduRequest::RequestSwitchInfo);
        }

        pub fn verify_switch_req_was_sent(
            &self,
            expected_queue_len: usize,
            switch_id: PcduSwitch,
            target_state: SwitchStateBinary,
        ) {
            // Check that there is now communication happening.
            let mut send_queue_mut = self.handler.com_interface.send_queue.borrow_mut();
            assert_eq!(send_queue_mut.len(), expected_queue_len);
            let packet_sent = send_queue_mut.pop_front().unwrap();
            drop(send_queue_mut);
            let pcdu_req: PcduRequest = serde_json::from_slice(&packet_sent).unwrap();
            assert_eq!(
                pcdu_req,
                PcduRequest::SwitchDevice {
                    switch: switch_id,
                    state: target_state
                }
            )
        }

        pub fn verify_switch_reply_received(
            &self,
            expected_queue_len: usize,
            expected_map: SwitchMapBinary,
        ) {
            // Check that a switch reply was read back.
            let mut reply_received_mut = self.handler.com_interface.reply_queue.borrow_mut();
            assert_eq!(reply_received_mut.len(), expected_queue_len);
            let reply_received = reply_received_mut.pop_front().unwrap();
            let sim_reply: SimReply = serde_json::from_str(&reply_received).unwrap();
            let pcdu_reply = PcduReply::from_sim_message(&sim_reply).unwrap();
            assert_eq!(pcdu_reply, PcduReply::SwitchInfo(expected_map));
        }
    }

    #[test]
    fn test_basic_handler() {
        let mut testbench = PcduTestbench::new();
        assert_eq!(testbench.handler.com_interface.send_queue.borrow().len(), 0);
        assert_eq!(
            testbench.handler.com_interface.reply_queue.borrow().len(),
            0
        );
        assert_eq!(
            testbench.handler.mode_and_submode().mode(),
            DeviceMode::Off as u32
        );
        assert_eq!(testbench.handler.mode_and_submode().submode(), 0_u16);
        testbench.handler.periodic_operation(OpCode::RegularOp);
        testbench
            .handler
            .periodic_operation(OpCode::PollAndRecvReplies);
        // Handler is OFF, no changes expected.
        assert_eq!(testbench.handler.com_interface.send_queue.borrow().len(), 0);
        assert_eq!(
            testbench.handler.com_interface.reply_queue.borrow().len(),
            0
        );
        assert_eq!(
            testbench.handler.mode_and_submode().mode(),
            DeviceMode::Off as u32
        );
        assert_eq!(testbench.handler.mode_and_submode().submode(), 0_u16);
    }

    #[test]
    fn test_normal_mode() {
        let mut testbench = PcduTestbench::new();
        testbench
            .mode_request_tx
            .send(GenericMessage::new(
                MessageMetadata::new(0, PUS_MODE_SERVICE.id()),
                ModeRequest::SetMode {
                    mode_and_submode: ModeAndSubmode::new(DeviceMode::Normal as u32, 0),
                    forced: false,
                },
            ))
            .expect("failed to send mode request");
        let switch_map_shared = testbench.handler.shared_switch_map.lock().unwrap();
        assert!(!switch_map_shared.valid);
        drop(switch_map_shared);
        testbench.handler.periodic_operation(OpCode::RegularOp);
        testbench
            .handler
            .periodic_operation(OpCode::PollAndRecvReplies);
        // Check correctness of mode.
        assert_eq!(
            testbench.handler.mode_and_submode().mode(),
            DeviceMode::Normal as u32
        );
        assert_eq!(testbench.handler.mode_and_submode().submode(), 0);

        testbench.verify_switch_info_req_was_sent(1);
        testbench.verify_switch_reply_received(1, SwitchMapBinaryWrapper::default().0);

        let switch_map_shared = testbench.handler.shared_switch_map.lock().unwrap();
        assert!(switch_map_shared.valid);
        drop(switch_map_shared);
    }

    #[test]
    fn test_switch_request_handling() {
        let mut testbench = PcduTestbench::new();
        testbench
            .mode_request_tx
            .send(GenericMessage::new(
                MessageMetadata::new(0, PUS_MODE_SERVICE.id()),
                ModeRequest::SetMode {
                    mode_and_submode: ModeAndSubmode::new(DeviceMode::Normal as u32, 0),
                    forced: false,
                },
            ))
            .expect("failed to send mode request");
        testbench
            .switch_request_tx
            .send(GenericMessage::new(
                MessageMetadata::new(0, MGM_HANDLER_0.id()),
                SwitchRequest::new(0, SwitchStateBinary::On),
            ))
            .expect("failed to send switch request");
        testbench.handler.periodic_operation(OpCode::RegularOp);
        testbench
            .handler
            .periodic_operation(OpCode::PollAndRecvReplies);

        testbench.verify_switch_req_was_sent(2, PcduSwitch::Mgm, SwitchStateBinary::On);
        testbench.verify_switch_info_req_was_sent(1);
        let mut switch_map = SwitchMapBinaryWrapper::default().0;
        *switch_map
            .get_mut(&PcduSwitch::Mgm)
            .expect("switch state setting failed") = SwitchStateBinary::On;
        testbench.verify_switch_reply_received(1, switch_map);

        let switch_map_shared = testbench.handler.shared_switch_map.lock().unwrap();
        assert!(switch_map_shared.valid);
        drop(switch_map_shared);
    }
}
