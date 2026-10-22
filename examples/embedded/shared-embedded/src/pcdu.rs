use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, ThreadModeRawMutex};
use minisim_types::{
    SimReply, SimRequestWithTime,
    eps::{PcduReply, PcduRequest},
};
use spacepackets::CcsdsPacketIdAndPsc;
use types::{
    ComponentId, DeviceMode,
    ccsds::{CcsdsTcPacketOwned, CcsdsTmPacketOwned},
    pcdu::{
        self, SwitchId, SwitchMapBinaryWrapper, SwitchMapWrapper, SwitchRequest, SwitchSet,
        SwitchStateBinary, SwitchesBitfield,
    },
};

pub type SharedSwitchSet = embassy_sync::mutex::Mutex<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    SwitchSet,
>;

//use crate::ccsds::pack_ccsds_tm_packet_for_now;

//pub type SharedSwitchSet = Arc<Mutex<SwitchSet>>;

/*
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

pub struct SerialInterfaceToSim {
    pub sim_request_tx: mpsc::Sender<SimRequestWithTime>,
    pub sim_reply_rx: mpsc::Receiver<SimReply>,
}

impl SerialInterfaceToSim {
    pub fn new(
        sim_request_tx: mpsc::Sender<SimRequestWithTime>,
        sim_reply_rx: mpsc::Receiver<SimReply>,
    ) -> Self {
        Self {
            sim_request_tx,
            sim_reply_rx,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[repr(u32)]
pub enum SetId {
    SwitcherSet = 0,
}

impl SerialInterface for SerialInterfaceToSim {
    type Error = ();

    fn send(&self, data: &[u8]) -> Result<(), Self::Error> {
        let request: PcduRequest = postcard::from_bytes(data).expect("expected a PCDU request");
        self.sim_request_tx
            .send(SimRequestWithTime::new_with_epoch_time(request))
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
                    let reply = postcard::to_allocvec(&reply).unwrap();
                    f(&reply);
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
        let pcdu_req: PcduRequest = postcard::from_bytes(data).unwrap();
        let switch_map_mut = &mut self.switch_map.borrow_mut().0;
        match pcdu_req {
            PcduRequest::SwitchDevice { switch, state } => {
                switch_map_mut
                    .insert(switch, state)
                    .expect("switch map capacity exceeded");
            }
            PcduRequest::RequestSwitchInfo => {
                let mut reply_deque_mut = self.reply_deque.borrow_mut();
                reply_deque_mut.push_back(SimReply::from(PcduReply::SwitchInfo(
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
            let reply = self.next_reply_serialized();
            f(&reply);
            if self.reply_queue_empty() {
                break;
            }
        }
        Ok(())
    }
}

impl SerialInterfaceDummy {
    fn next_reply_serialized(&self) -> Vec<u8> {
        let mut reply_deque_mut = self.reply_deque.borrow_mut();
        let next_reply = reply_deque_mut.pop_front().unwrap();
        postcard::to_allocvec(&next_reply).unwrap()
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
*/

pub const TC_QUEUE_DEPTH: usize = 16;
pub const TM_QUEUE_DEPTH: usize = 16;
pub const SWITCH_REQ_QUEUE_DEPTH: usize = 32;
pub const EVENT_QUEUE_DEPTH: usize = 8;

/// Example PCDU device handler.
#[allow(clippy::too_many_arguments)]
pub struct PcduHandler {
    dev_str: &'static str,
    switch_request_rx: embassy_sync::channel::Receiver<
        'static,
        CriticalSectionRawMutex,
        SwitchRequest,
        SWITCH_REQ_QUEUE_DEPTH,
    >,
    tc_rx: embassy_sync::channel::Receiver<
        'static,
        ThreadModeRawMutex,
        CcsdsTcPacketOwned,
        TC_QUEUE_DEPTH,
    >,
    tm_tx: embassy_sync::channel::Sender<
        'static,
        ThreadModeRawMutex,
        CcsdsTmPacketOwned,
        TM_QUEUE_DEPTH,
    >,
    //pub com_interface: ComInterface,
    shared_switch_map: SharedSwitchSet,
    mode: DeviceMode,
    //stamp_helper: TimestampHelper,
    event_tx:
        embassy_sync::channel::Sender<'static, ThreadModeRawMutex, pcdu::Event, EVENT_QUEUE_DEPTH>,
}

/*
impl<ComInterface: SerialInterface> PcduHandler<ComInterface> {
    pub fn new(
        tc_rx: std::sync::mpsc::Receiver<CcsdsTcPacketOwned>,
        tm_tx: std::sync::mpsc::SyncSender<CcsdsTmPacketOwned>,
        switch_request_rx: mpsc::Receiver<SwitchRequest>,
        com_interface: ComInterface,
        shared_switch_map: Arc<Mutex<SwitchSet>>,
        init_mode: DeviceMode,
        event_tx: mpsc::SyncSender<pcdu::Event>,
    ) -> Self {
        Self {
            dev_str: "PCDU",
            tc_rx,
            switch_request_rx,
            tm_tx,
            com_interface,
            shared_switch_map,
            stamp_helper: TimestampHelper::default(),
            // Start in normal mode by default. Assume that the PCDU itself is on by default.
            mode: init_mode,
            event_tx,
        }
    }

    pub fn periodic_operation(&mut self, op_code: OpCode) {
        match op_code {
            OpCode::RegularOp => {
                self.stamp_helper.update_from_now();
                // Handle requests.
                self.handle_telecommands();
                self.handle_switch_requests();
                // Poll the switch states and/or telemetry regularly here.
                if self.mode() == DeviceMode::Normal || self.mode() == DeviceMode::On {
                    self.handle_periodic_commands();
                }
            }
            OpCode::PollAndRecvReplies => {
                self.poll_and_handle_replies();
            }
        }
    }

    #[inline]
    pub fn mode(&self) -> DeviceMode {
        self.mode
    }

    pub fn handle_telecommands(&mut self) {
        loop {
            match self.tc_rx.try_recv() {
                Ok(packet) => {
                    let tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&packet.sp_header);
                    match postcard::from_bytes::<pcdu::request::Request>(&packet.payload) {
                        Ok(request) => {
                            log::info!(
                                "received request {:?} with TC ID {:#010x}",
                                request,
                                tc_id.raw()
                            );
                            match request {
                                pcdu::request::Request::Ping => {
                                    self.send_tm(Some(tc_id), pcdu::response::Response::Ok)
                                }
                                pcdu::request::Request::GetSwitches => self.send_tm(
                                    Some(tc_id),
                                    pcdu::response::Response::Switches(
                                        self.shared_switch_map
                                            .lock()
                                            .unwrap()
                                            .as_bitfield()
                                            .expect("could not build switches response"),
                                    ),
                                ),
                                pcdu::request::Request::EnableSwitches(switches) => {
                                    self.handle_switches_bitfield_request(
                                        switches,
                                        SwitchStateBinary::On,
                                    );
                                }
                                pcdu::request::Request::DisableSwitches(switches) => {
                                    self.handle_switches_bitfield_request(
                                        switches,
                                        SwitchStateBinary::Off,
                                    );
                                }
                                pcdu::request::Request::Mode(device_mode) => {
                                    self.switch_mode(tc_id, device_mode)
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("failed to deserialize request: {}", e);
                        }
                    }
                }
                Err(e) => match e {
                    std::sync::mpsc::TryRecvError::Empty => break,
                    std::sync::mpsc::TryRecvError::Disconnected => {
                        log::warn!("packet sender disconnected")
                    }
                },
            }
        }
    }

    pub fn handle_switches_bitfield_request(
        &mut self,
        switches: SwitchesBitfield,
        state: SwitchStateBinary,
    ) {
        if switches.mgm0() {
            self.handle_device_switching(SwitchId::Mgm0, state);
        }
        if switches.mgm1() {
            self.handle_device_switching(SwitchId::Mgm1, state);
        }
        if switches.magnetorquer() {
            self.handle_device_switching(SwitchId::Mgt, state);
        }
    }

    pub fn send_tm(&self, tc_id: Option<CcsdsPacketIdAndPsc>, response: pcdu::response::Response) {
        match pack_ccsds_tm_packet_for_now(ComponentId::EpsPcdu, tc_id, &response) {
            Ok(packet) => {
                if let Err(e) = self.tm_tx.send(packet) {
                    log::warn!("failed to send TM packet: {}", e);
                }
            }
            Err(e) => {
                log::warn!("failed to pack TM packet: {}", e);
            }
        }
    }

    fn switch_mode(&mut self, requestor: CcsdsPacketIdAndPsc, mode: DeviceMode) {
        log::info!("{}: transitioning to mode {:?}", self.dev_str, mode);
        self.mode = mode;
        if self.mode() == DeviceMode::Off {
            self.shared_switch_map.lock().unwrap().valid = false;
        }
        log::info!("{} announcing mode: {:?}", self.dev_str, self.mode);
        self.send_telemetry(Some(requestor), pcdu::response::Response::Ok);
    }

    pub fn send_telemetry(
        &self,
        tc_id: Option<CcsdsPacketIdAndPsc>,
        response: pcdu::response::Response,
    ) {
        match pack_ccsds_tm_packet_for_now(ComponentId::EpsPcdu, tc_id, &response) {
            Ok(packet) => {
                if let Err(e) = self.tm_tx.send(packet) {
                    log::warn!("failed to send TM packet: {}", e);
                }
            }
            Err(e) => {
                log::warn!("failed to pack TM packet: {}", e);
            }
        }
    }

    pub fn handle_periodic_commands(&self) {
        let pcdu_req = PcduRequest::RequestSwitchInfo;
        let pcdu_req_ser = postcard::to_allocvec(&pcdu_req).unwrap();
        if let Err(_e) = self.com_interface.send(&pcdu_req_ser) {
            log::warn!("polling PCDU switch info failed");
            if let Err(e) = self.event_tx.send(pcdu::Event::SerialCommError) {
                log::warn!("failed to send comm error event: {}", e);
            }
        }
    }

    /*
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
    */

    pub fn handle_device_switching(&mut self, switch_id: SwitchId, state: SwitchStateBinary) {
        let pcdu_req = PcduRequest::SwitchDevice {
            switch: switch_id,
            state,
        };
        let pcdu_req_ser = postcard::to_allocvec(&pcdu_req).unwrap();
        self.com_interface
            .send(&pcdu_req_ser)
            .expect("failed to send switch request to PCDU");
    }

    pub fn handle_switch_requests(&mut self) {
        loop {
            match self.switch_request_rx.try_recv() {
                Ok(switch_req) => {
                    self.handle_device_switching(switch_req.switch_id(), switch_req.target_state());
                }
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
            let sim_reply: SimReply = postcard::from_bytes(reply).expect("invalid reply format");
            let SimReply::Pcdu(pcdu_reply) = sim_reply else {
                log::warn!("unexpected PCDU SIM reply: {sim_reply:?}");
                return;
            };
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
            log::warn!("receiving PCDU replies failed: {e:?}");
        }
    }
}
*/
