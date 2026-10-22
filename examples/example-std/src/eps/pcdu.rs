use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, mpsc},
};

use example_std::TimestampHelper;
use minisim_types::{
    SimReply, SimRequestWithTime,
    eps::{PcduReply, PcduRequest},
};
use satrs::spacepackets::CcsdsPacketIdAndPsc;
use types::{
    ComponentId, DeviceMode,
    ccsds::{CcsdsTcPacketOwned, CcsdsTmPacketOwned},
    pcdu::{
        self, SwitchId, SwitchMapBinaryWrapper, SwitchMapWrapper, SwitchRequest, SwitchSet,
        SwitchStateBinary, SwitchesBitfield,
    },
};

use crate::ccsds::pack_ccsds_tm_packet_for_now;

pub type SharedSwitchSet = Arc<Mutex<SwitchSet>>;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CommError;

pub struct SimInterface {
    pub sim_request_tx: mpsc::Sender<SimRequestWithTime>,
    pub sim_reply_rx: mpsc::Receiver<SimReply>,
}

impl SimInterface {
    pub fn new(
        sim_request_tx: mpsc::Sender<SimRequestWithTime>,
        sim_reply_rx: mpsc::Receiver<SimReply>,
    ) -> Self {
        Self {
            sim_request_tx,
            sim_reply_rx,
        }
    }

    fn send(&mut self, request: PcduRequest) -> Result<(), CommError> {
        self.sim_request_tx
            .send(SimRequestWithTime::new_with_epoch_time(request))
            .map_err(|_| CommError)
    }

    fn try_recv_reply(&mut self) -> Option<PcduReply> {
        loop {
            match self.sim_reply_rx.try_recv() {
                Ok(SimReply::Pcdu(reply)) => return Some(reply),
                Ok(sim_reply) => log::warn!("unexpected PCDU SIM reply: {sim_reply:?}"),
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    log::warn!("sim reply sender has disconnected");
                    return None;
                }
            }
        }
    }
}

/// Interface for an ideal PCDU which never fails.
#[derive(Default)]
pub struct DummyInterface {
    switch_map: SwitchMapBinaryWrapper,
    replies: VecDeque<PcduReply>,
}

impl DummyInterface {
    fn send(&mut self, request: PcduRequest) {
        match request {
            PcduRequest::SwitchDevice { switch, state } => {
                self.switch_map
                    .0
                    .insert(switch, state)
                    .expect("switch map capacity exceeded");
            }
            PcduRequest::RequestSwitchInfo => self
                .replies
                .push_back(PcduReply::SwitchInfo(self.switch_map.0.clone())),
        }
    }
}

/// Wraps the dummy interface. It records all requests and replies and can make sending fail.
#[derive(Default)]
pub struct TestInterface {
    pub dummy: DummyInterface,
    pub sent_requests: VecDeque<PcduRequest>,
    pub received_replies: VecDeque<PcduReply>,
    pub fail_next_send: bool,
}

pub enum PcduCommunication {
    Dummy(DummyInterface),
    Sim(SimInterface),
    #[allow(dead_code)]
    Test(TestInterface),
}

impl PcduCommunication {
    fn send(&mut self, request: PcduRequest) -> Result<(), CommError> {
        match self {
            PcduCommunication::Dummy(dummy) => {
                dummy.send(request);
                Ok(())
            }
            PcduCommunication::Sim(sim) => sim.send(request),
            PcduCommunication::Test(test) => {
                if core::mem::take(&mut test.fail_next_send) {
                    return Err(CommError);
                }
                test.sent_requests.push_back(request);
                test.dummy.send(request);
                Ok(())
            }
        }
    }

    fn try_recv_reply(&mut self) -> Option<PcduReply> {
        match self {
            PcduCommunication::Dummy(dummy) => dummy.replies.pop_front(),
            PcduCommunication::Sim(sim) => sim.try_recv_reply(),
            PcduCommunication::Test(test) => {
                let reply = test.dummy.replies.pop_front()?;
                test.received_replies.push_back(reply.clone());
                Some(reply)
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum OpCode {
    RegularOp = 0,
    PollAndRecvReplies = 1,
}

/// Example PCDU device handler.
#[allow(clippy::too_many_arguments)]
pub struct PcduHandler {
    dev_str: &'static str,
    switch_request_rx: mpsc::Receiver<SwitchRequest>,
    tc_rx: std::sync::mpsc::Receiver<CcsdsTcPacketOwned>,
    tm_tx: mpsc::SyncSender<CcsdsTmPacketOwned>,
    pub com: PcduCommunication,
    shared_switch_map: SharedSwitchSet,
    mode: DeviceMode,
    stamp_helper: TimestampHelper,
    event_tx: mpsc::SyncSender<pcdu::Event>,
}

impl PcduHandler {
    pub fn new(
        tc_rx: std::sync::mpsc::Receiver<CcsdsTcPacketOwned>,
        tm_tx: std::sync::mpsc::SyncSender<CcsdsTmPacketOwned>,
        switch_request_rx: mpsc::Receiver<SwitchRequest>,
        com: PcduCommunication,
        shared_switch_map: SharedSwitchSet,
        init_mode: DeviceMode,
        event_tx: mpsc::SyncSender<pcdu::Event>,
    ) -> Self {
        Self {
            dev_str: "PCDU",
            tc_rx,
            switch_request_rx,
            tm_tx,
            com,
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

    pub fn handle_periodic_commands(&mut self) {
        if self.com.send(PcduRequest::RequestSwitchInfo).is_err() {
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
        self.com
            .send(pcdu_req)
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
        while let Some(reply) = self.com.try_recv_reply() {
            match reply {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use arbitrary_int::u11;
    use satrs::spacepackets::SpacePacketHeader;
    use types::{
        Apid, TcHeader,
        pcdu::{SwitchMapBinary, SwitchStateBinary},
    };

    use super::*;

    pub fn create_request_tc(
        request: types::pcdu::request::Request,
    ) -> types::ccsds::CcsdsTcPacketOwned {
        types::ccsds::CcsdsTcPacketOwned::new_with_request(
            SpacePacketHeader::new_from_apid(u11::new(Apid::Eps as u16)),
            TcHeader::new(ComponentId::EpsPcdu, request.message_type()),
            request,
        )
    }

    #[allow(dead_code)]
    pub struct PcduTestbench {
        pub mode_request_tx: mpsc::SyncSender<types::pcdu::request::Request>,
        pub mode_reply_rx_to_parent: mpsc::Receiver<types::pcdu::response::Response>,
        pub tc_tx: mpsc::SyncSender<CcsdsTcPacketOwned>,
        pub tm_rx: mpsc::Receiver<CcsdsTmPacketOwned>,
        pub switch_request_tx: mpsc::Sender<SwitchRequest>,
        pub event_rx: mpsc::Receiver<pcdu::Event>,
        pub handler: PcduHandler,
    }

    impl PcduTestbench {
        pub fn new() -> Self {
            let (mode_request_tx, _mode_request_rx) = mpsc::sync_channel(5);
            let (_mode_reply_tx_to_parent, mode_reply_rx_to_parent) = mpsc::sync_channel(5);
            let (tc_tx, tc_rx) = mpsc::sync_channel(5);
            let (tm_tx, tm_rx) = mpsc::sync_channel(5);
            let (switch_request_tx, switch_reqest_rx) = mpsc::channel();
            let (event_tx, event_rx) = mpsc::sync_channel(5);
            let shared_switch_map =
                Arc::new(Mutex::new(SwitchSet::new_with_init_switches_unknown()));
            let handler = PcduHandler::new(
                tc_rx,
                tm_tx.clone(),
                switch_reqest_rx,
                PcduCommunication::Test(TestInterface::default()),
                shared_switch_map,
                DeviceMode::Off,
                event_tx,
            );
            Self {
                mode_request_tx,
                mode_reply_rx_to_parent,
                tc_tx,
                tm_rx,
                switch_request_tx,
                event_rx,
                handler,
            }
        }

        fn test_interface(&mut self) -> &mut TestInterface {
            match &mut self.handler.com {
                PcduCommunication::Test(test) => test,
                _ => panic!("unexpected PCDU interface"),
            }
        }

        pub fn verify_switch_info_req_was_sent(&mut self, expected_queue_len: usize) {
            let sent_requests = &mut self.test_interface().sent_requests;
            assert_eq!(sent_requests.len(), expected_queue_len);
            assert_eq!(
                sent_requests.pop_front().unwrap(),
                PcduRequest::RequestSwitchInfo
            );
        }

        pub fn verify_switch_req_was_sent(
            &mut self,
            expected_queue_len: usize,
            switch_id: SwitchId,
            target_state: SwitchStateBinary,
        ) {
            let sent_requests = &mut self.test_interface().sent_requests;
            assert_eq!(sent_requests.len(), expected_queue_len);
            assert_eq!(
                sent_requests.pop_front().unwrap(),
                PcduRequest::SwitchDevice {
                    switch: switch_id,
                    state: target_state
                }
            )
        }

        pub fn verify_switch_reply_received(
            &mut self,
            expected_queue_len: usize,
            expected_map: SwitchMapBinary,
        ) {
            let received_replies = &mut self.test_interface().received_replies;
            assert_eq!(received_replies.len(), expected_queue_len);
            assert_eq!(
                received_replies.pop_front().unwrap(),
                PcduReply::SwitchInfo(expected_map)
            );
        }
    }

    #[test]
    fn test_periodic_command_send_failure_sends_event() {
        let mut testbench = PcduTestbench::new();
        testbench.test_interface().fail_next_send = true;
        testbench.handler.handle_periodic_commands();
        let event = testbench
            .event_rx
            .try_recv()
            .expect("expected comm error event");
        assert!(matches!(event, pcdu::Event::SerialCommError));
    }

    #[test]
    fn test_basic_handler() {
        let mut testbench = PcduTestbench::new();
        assert!(testbench.test_interface().sent_requests.is_empty());
        assert!(testbench.test_interface().received_replies.is_empty());
        assert_eq!(testbench.handler.mode(), DeviceMode::Off);
        testbench.handler.periodic_operation(OpCode::RegularOp);
        testbench
            .handler
            .periodic_operation(OpCode::PollAndRecvReplies);
        // Handler is OFF, no changes expected.
        assert!(testbench.test_interface().sent_requests.is_empty());
        assert!(testbench.test_interface().received_replies.is_empty());
        assert_eq!(testbench.handler.mode(), DeviceMode::Off);
    }

    #[test]
    fn test_normal_mode() {
        let mut testbench = PcduTestbench::new();
        testbench
            .tc_tx
            .send(create_request_tc(pcdu::request::Request::Mode(
                DeviceMode::Normal,
            )))
            .unwrap();
        let switch_map_shared = testbench.handler.shared_switch_map.lock().unwrap();
        assert!(switch_map_shared.valid);
        drop(switch_map_shared);
        testbench.handler.periodic_operation(OpCode::RegularOp);
        testbench
            .handler
            .periodic_operation(OpCode::PollAndRecvReplies);
        // Check correctness of mode.
        assert_eq!(testbench.handler.mode(), DeviceMode::Normal);

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
            .tc_tx
            .send(create_request_tc(pcdu::request::Request::Mode(
                DeviceMode::Normal,
            )))
            .unwrap();
        testbench
            .switch_request_tx
            .send(SwitchRequest::new(SwitchId::Mgm0, SwitchStateBinary::On))
            .expect("failed to send switch request");
        testbench.handler.periodic_operation(OpCode::RegularOp);
        testbench
            .handler
            .periodic_operation(OpCode::PollAndRecvReplies);

        testbench.verify_switch_req_was_sent(2, SwitchId::Mgm0, SwitchStateBinary::On);
        testbench.verify_switch_info_req_was_sent(1);
        let mut switch_map = SwitchMapBinaryWrapper::default().0;
        *switch_map
            .get_mut(&SwitchId::Mgm0)
            .expect("switch state setting failed") = SwitchStateBinary::On;
        testbench.verify_switch_reply_received(1, switch_map);

        let switch_map_shared = testbench.handler.shared_switch_map.lock().unwrap();
        assert!(switch_map_shared.valid);
        drop(switch_map_shared);
    }
}
