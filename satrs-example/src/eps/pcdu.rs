use std::{
    cell::RefCell,
    collections::VecDeque,
    sync::{mpsc, Arc, Mutex},
};

use derive_new::new;
use satrs::{
    hk::{HkRequest, HkRequestVariant},
    mode::{ModeAndSubmode, ModeError, ModeProvider, ModeReply, ModeRequestHandler},
    power::{SwitchRequest, SwitchStateBinary},
    pus::EcssTmSender,
    queue::{GenericSendError, GenericTargetedMessagingError},
    request::{GenericMessage, MessageMetadata, UniqueApidTargetId},
};
use satrs_example::{config::components::PUS_MODE_SERVICE, DeviceMode, TimestampHelper};
use satrs_minisim::{
    eps::{PcduReply, PcduRequest, PcduSwitch, SwitchMap, SwitchMapBinary, SwitchMapBinaryWrapper},
    SerializableSimMsgPayload, SimReply, SimRequest,
};

use crate::{acs::mgm::MpscModeLeafInterface, pus::hk::HkReply, requests::CompositeRequest};

pub trait SerialInterface {
    type Error;
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

impl SerialInterface for SerialInterfaceToSim {
    type Error = ();

    fn send(&self, data: &[u8]) -> Result<(), Self::Error> {
        let request: SimRequest = serde_json::from_slice(data).unwrap();
        self.sim_request_tx
            .send(request)
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
        let sim_req: SimRequest = serde_json::from_slice(data).unwrap();
        let pcdu_req =
            PcduRequest::from_sim_message(&sim_req).expect("PCDU request creation failed");
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
                    self.switch_map.borrow().0.clone(),
                )));
            }
        };
        Ok(())
    }

    fn try_recv_replies<ReplyHandler: FnMut(&[u8])>(
        &self,
        mut f: ReplyHandler,
    ) -> Result<(), Self::Error> {
        if self.reply_deque.borrow().is_empty() {
            return Ok(());
        }
        loop {
            let mut reply_deque_mut = self.reply_deque.borrow_mut();
            let next_reply = reply_deque_mut.pop_front().unwrap();
            let reply = serde_json::to_string(&next_reply).unwrap();
            f(reply.as_bytes());
            if reply_deque_mut.is_empty() {
                break;
            }
        }
        Ok(())
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

#[derive(Clone, PartialEq, Eq, Default)]
pub struct SwitchSet {
    pub valid: bool,
    pub switch_map: SwitchMap,
}

pub type SharedSwitchSet = Arc<Mutex<SwitchSet>>;

/// Example PCDU device handler.
#[derive(new)]
#[allow(clippy::too_many_arguments)]
pub struct PcduHandler<ComInterface: SerialInterface, TmSender: EcssTmSender> {
    id: UniqueApidTargetId,
    dev_str: &'static str,
    mode_interface: MpscModeLeafInterface,
    composite_request_rx: mpsc::Receiver<GenericMessage<CompositeRequest>>,
    hk_reply_tx: mpsc::Sender<GenericMessage<HkReply>>,
    switch_request_rx: mpsc::Receiver<GenericMessage<SwitchRequest>>,
    tm_sender: TmSender,
    pub com_interface: ComInterface,
    shared_switch_map: Arc<Mutex<SwitchSet>>,
    #[new(value = "ModeAndSubmode::new(satrs_example::DeviceMode::Off as u32, 0)")]
    mode_and_submode: ModeAndSubmode,
    #[new(default)]
    stamp_helper: TimestampHelper,
}

impl<ComInterface: SerialInterface, TmSender: EcssTmSender> PcduHandler<ComInterface, TmSender> {
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
            HkRequestVariant::OneShot => todo!(),
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
            match self.mode_interface.request_rx.try_recv() {
                Ok(msg) => {
                    let result = self.handle_mode_request(msg);
                    // TODO: Trigger event?
                    if result.is_err() {
                        log::warn!(
                            "{}: mode request failed with error {:?}",
                            self.dev_str,
                            result.err().unwrap()
                        );
                    }
                }
                Err(e) => {
                    if e != mpsc::TryRecvError::Empty {
                        log::warn!("{}: failed to receive mode request: {:?}", self.dev_str, e);
                    } else {
                        break;
                    }
                }
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
                        self.com_interface.send(pcdu_req_ser.as_bytes())
                    }
                    Err(e) => todo!("failed to convert switch ID {:?} to typed PCDU switch", e),
                },
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => todo!(),
                    mpsc::TryRecvError::Disconnected => todo!(),
                },
            };
        }
    }

    pub fn poll_and_handle_replies(&mut self) {}
}

impl<ComInterface: SerialInterface, TmSender: EcssTmSender> ModeProvider
    for PcduHandler<ComInterface, TmSender>
{
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl<ComInterface: SerialInterface, TmSender: EcssTmSender> ModeRequestHandler
    for PcduHandler<ComInterface, TmSender>
{
    type Error = ModeError;
    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
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
        self.mode_interface
            .reply_to_pus_tx
            .send(GenericMessage::new(requestor, reply))
            .map_err(|_| GenericTargetedMessagingError::Send(GenericSendError::RxDisconnected))?;
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
