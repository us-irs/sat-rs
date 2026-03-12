use models::ccsds::{CcsdsTcPacketOwned, CcsdsTmPacketOwned};
use models::mgm::response::ModeFailure;
use models::mgm::MgmData;
use models::pcdu::SwitchId;
use models::{mgm, ComponentId, DeviceMode, HkRequestType};
use satrs::spacepackets::CcsdsPacketIdAndPsc;
use satrs_example::{HkHelperSingleSet, ModeHelper, TimestampHelper, TransitionState};
use satrs_minisim::acs::lis3mdl::{
    MgmLis3MdlReply, MgmLis3RawValues, FIELD_LSB_PER_GAUSS_4_SENS, GAUSS_TO_MICROTESLA_FACTOR,
};
use satrs_minisim::acs::MgmRequestLis3Mdl;
use satrs_minisim::{SerializableSimMsgPayload, SimReply, SimRequest};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use satrs::request::MessageMetadata;

use crate::ccsds::pack_ccsds_tm_packet_for_now;
use crate::eps::PowerSwitchHelper;

pub const NR_OF_DATA_AND_CFG_REGISTERS: usize = 14;

// Register adresses to access various bytes from the raw reply.
pub const X_LOWBYTE_IDX: usize = 9;
pub const Y_LOWBYTE_IDX: usize = 11;
pub const Z_LOWBYTE_IDX: usize = 13;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum MgmId {
    _0,
    _1,
}

impl MgmId {
    pub const fn str(&self) -> &str {
        match self {
            MgmId::_0 => "MGM 0",
            MgmId::_1 => "MGM 1",
        }
    }

    #[inline]
    pub const fn component_id(&self) -> ComponentId {
        match self {
            MgmId::_0 => ComponentId::AcsMgm0,
            MgmId::_1 => ComponentId::AcsMgm1,
        }
    }
}

pub enum ModeRequest {
    SetMode(DeviceMode),
    ReadMode,
}

pub enum ModeReport {
    Mode(DeviceMode),
    /// Setting a mode timed out.
    SetModeTimeout,
}

#[derive(Default)]
pub struct SpiDummyInterface {
    pub dummy_values: MgmLis3RawValues,
}

impl SpiDummyInterface {
    fn transfer(&mut self, _tx: &[u8], rx: &mut [u8]) {
        rx[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2].copy_from_slice(&self.dummy_values.x.to_le_bytes());
        rx[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2].copy_from_slice(&self.dummy_values.y.to_be_bytes());
        rx[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2].copy_from_slice(&self.dummy_values.z.to_be_bytes());
    }
}

pub struct SpiSimInterface {
    pub sim_request_tx: mpsc::Sender<SimRequest>,
    pub sim_reply_rx: mpsc::Receiver<SimReply>,
}

impl SpiSimInterface {
    // Right now, we only support requesting sensor data and not configuration of the sensor.
    fn transfer(&mut self, _tx: &[u8], rx: &mut [u8]) {
        let mgm_sensor_request = MgmRequestLis3Mdl::RequestSensorData;
        if let Err(e) = self
            .sim_request_tx
            .send(SimRequest::new_with_epoch_time(mgm_sensor_request))
        {
            log::error!("failed to send MGM LIS3 request: {e}");
        }
        match self.sim_reply_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(sim_reply) => {
                let sim_reply_lis3 = MgmLis3MdlReply::from_sim_message(&sim_reply)
                    .expect("failed to parse LIS3 reply");
                rx[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2]
                    .copy_from_slice(&sim_reply_lis3.raw.x.to_le_bytes());
                rx[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2]
                    .copy_from_slice(&sim_reply_lis3.raw.y.to_le_bytes());
                rx[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2]
                    .copy_from_slice(&sim_reply_lis3.raw.z.to_le_bytes());
            }
            Err(e) => {
                log::warn!("MGM LIS3 SIM reply timeout: {e}");
            }
        }
    }
}

pub enum SpiCommunication {
    Dummy(SpiDummyInterface),
    Sim(SpiSimInterface),
}

impl SpiCommunication {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) {
        match self {
            SpiCommunication::Dummy(dummy) => dummy.transfer(tx, rx),
            SpiCommunication::Sim(sim_if) => sim_if.transfer(tx, rx),
        }
    }
}

#[derive(Default)]
pub struct BufWrapper {
    tx_buf: [u8; 32],
    rx_buf: [u8; 32],
}

/// Helper component for communication with a parent component, which is usually as assembly.
pub struct ModeLeafHelper {
    pub request_rx: mpsc::Receiver<ModeRequest>,
    pub report_tx: mpsc::SyncSender<ModeReport>,
}

/// Example MGM device handler strongly based on the LIS3MDL MEMS device.
pub struct MgmHandlerLis3Mdl {
    id: MgmId,
    tc_rx: mpsc::Receiver<CcsdsTcPacketOwned>,
    tm_tx: mpsc::SyncSender<CcsdsTmPacketOwned>,
    switch_helper: PowerSwitchHelper,
    pub com_interface: SpiCommunication,
    shared_mgm_set: Arc<Mutex<MgmData>>,
    buffers: BufWrapper,
    stamp_helper: TimestampHelper,
    hk_helper: HkHelperSingleSet,
    mode_helpers: ModeHelper<DeviceMode>,
    mode_leaf_helper: ModeLeafHelper,
}

impl MgmHandlerLis3Mdl {
    pub fn new(
        id: MgmId,
        tc_rx: mpsc::Receiver<CcsdsTcPacketOwned>,
        tm_tx: mpsc::SyncSender<CcsdsTmPacketOwned>,
        switch_helper: PowerSwitchHelper,
        com_interface: SpiCommunication,
        shared_mgm_set: Arc<Mutex<MgmData>>,
        mode_leaf_helper: ModeLeafHelper,
    ) -> Self {
        Self {
            id,
            tc_rx,
            tm_tx,
            switch_helper,
            com_interface,
            shared_mgm_set,
            mode_helpers: ModeHelper::new(DeviceMode::Off, Duration::from_millis(200)),
            buffers: BufWrapper::default(),
            stamp_helper: TimestampHelper::default(),
            hk_helper: HkHelperSingleSet::new(false, Duration::from_millis(200)),
            mode_leaf_helper,
        }
    }

    #[inline]
    pub fn mode(&self) -> DeviceMode {
        self.mode_helpers.current
    }

    #[inline]
    pub fn switch_id(&self) -> SwitchId {
        match self.id {
            MgmId::_0 => SwitchId::Mgm0,
            MgmId::_1 => SwitchId::Mgm1,
            _ => panic!("unexpected component id"),
        }
    }

    pub fn periodic_operation(&mut self) {
        // Update current time.
        self.stamp_helper.update_from_now();

        // Handle requests.
        self.handle_telecommands();

        // Handle assembly related messages.
        self.handle_mode_leaf_handling();

        // Handle mode transitions first.
        self.handle_mode_transition();

        // Poll sensor before checking and generating HK.
        if self.mode() == DeviceMode::Normal {
            log::trace!("polling LIS3MDL sensor {}", self.id.str());
            self.poll_sensor();
        }

        // Finally check whether any HK generation is necessary.
        if self.hk_helper.needs_generation() {
            self.generate_hk(None);
        }
    }

    pub fn handle_telecommands(&mut self) {
        loop {
            match self.tc_rx.try_recv() {
                Ok(packet) => {
                    let tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&packet.sp_header);
                    match postcard::from_bytes::<mgm::request::Request>(&packet.payload) {
                        Ok(request) => {
                            log::info!(
                                "received request {:?} with TC ID {:#010x}",
                                request,
                                tc_id.raw()
                            );
                            match request {
                                mgm::request::Request::Ping => {
                                    self.send_telemetry(Some(tc_id), mgm::response::Response::Ok)
                                }
                                mgm::request::Request::Hk(hk_request) => {
                                    self.handle_hk_request(Some(tc_id), &hk_request)
                                }
                                mgm::request::Request::Mode(device_mode) => {
                                    self.mode_helpers.tc_id = Some(tc_id);
                                    self.start_transition(device_mode, false);
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

    pub fn handle_mode_leaf_handling(&mut self) {
        loop {
            match self.mode_leaf_helper.request_rx.try_recv() {
                Ok(request) => match request {
                    ModeRequest::SetMode(device_mode) => self.start_transition(device_mode, false),
                    ModeRequest::ReadMode => self.report_mode_to_parent(),
                },
                Err(e) => match e {
                    std::sync::mpsc::TryRecvError::Empty => break,
                    std::sync::mpsc::TryRecvError::Disconnected => {
                        log::warn!("packet sender disconnected")
                    }
                },
            }
        }
    }

    pub fn send_telemetry(
        &self,
        tc_id: Option<CcsdsPacketIdAndPsc>,
        response: mgm::response::Response,
    ) {
        match pack_ccsds_tm_packet_for_now(self.id.component_id(), tc_id, &response) {
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

    pub fn handle_hk_request(
        &mut self,
        tc_id: Option<CcsdsPacketIdAndPsc>,
        hk_request: &models::mgm::request::HkRequest,
    ) {
        match hk_request.req_type {
            HkRequestType::OneShot => {
                self.generate_hk(tc_id);
            }
            HkRequestType::EnablePeriodic(duration) => {
                self.hk_helper.enabled = true;
                self.hk_helper.frequency = duration;
            }
            HkRequestType::DisablePeriodic => {
                self.hk_helper.enabled = false;
            }
            HkRequestType::ModifyInterval(duration) => {
                self.hk_helper.frequency = duration;
            }
            _ => log::warn!("unhandled HK request"),
        }
    }

    pub fn generate_hk(&self, opt_tc_id: Option<CcsdsPacketIdAndPsc>) {
        let mgm_snapshot = *self.shared_mgm_set.lock().unwrap();
        self.send_telemetry(
            opt_tc_id,
            mgm::response::Response::Hk(mgm::response::HkResponse::MgmData(mgm_snapshot)),
        )
    }

    pub fn poll_sensor(&mut self) {
        // Communicate with the device. This is actually how to read the data from the LIS3 device
        // SPI interface.
        self.com_interface
            .transfer(
                &self.buffers.tx_buf[0..NR_OF_DATA_AND_CFG_REGISTERS + 1],
                &mut self.buffers.rx_buf[0..NR_OF_DATA_AND_CFG_REGISTERS + 1],
            )
            .expect("failed to transfer data");
        let x_raw = i16::from_le_bytes(
            self.buffers.rx_buf[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2]
                .try_into()
                .unwrap(),
        );
        let y_raw = i16::from_le_bytes(
            self.buffers.rx_buf[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2]
                .try_into()
                .unwrap(),
        );
        let z_raw = i16::from_le_bytes(
            self.buffers.rx_buf[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2]
                .try_into()
                .unwrap(),
        );
        // Simple scaling to retrieve the float value, assuming the best sensor resolution.
        let mut mgm_guard = self.shared_mgm_set.lock().unwrap();
        mgm_guard.x = x_raw as f32 * GAUSS_TO_MICROTESLA_FACTOR as f32 * FIELD_LSB_PER_GAUSS_4_SENS;
        mgm_guard.y = y_raw as f32 * GAUSS_TO_MICROTESLA_FACTOR as f32 * FIELD_LSB_PER_GAUSS_4_SENS;
        mgm_guard.z = z_raw as f32 * GAUSS_TO_MICROTESLA_FACTOR as f32 * FIELD_LSB_PER_GAUSS_4_SENS;
        mgm_guard.valid = true;
        drop(mgm_guard);
    }

    fn start_transition(&mut self, target_mode: DeviceMode, _forced: bool) {
        log::info!("{}: transitioning to mode {:?}", self.id.str(), target_mode);
        if target_mode == DeviceMode::Off {
            self.shared_mgm_set.lock().unwrap().valid = false;
        }
        self.mode_helpers.start(target_mode);
    }

    pub fn handle_mode_transition(&mut self) {
        if self.mode_helpers.target.is_none() {
            return;
        }
        let target_mode = self.mode_helpers.target.unwrap();
        if target_mode == DeviceMode::On || target_mode == DeviceMode::Normal {
            if self.mode_helpers.transition_state == TransitionState::Idle {
                let result = self
                    .switch_helper
                    .send_switch_on_cmd(MessageMetadata::new(0, self.id as u32), self.switch_id());
                if result.is_err() {
                    // Could not send switch command.. still continue with transition.
                    log::error!("failed to send switch on command");
                }
                self.mode_helpers.transition_state = TransitionState::PowerSwitching;
            }
            if self.mode_helpers.transition_state == TransitionState::PowerSwitching {
                if self.switch_helper.is_switch_on(self.switch_id()) {
                    self.mode_helpers.transition_state = TransitionState::Done;
                } else if self.mode_helpers.timed_out() {
                    self.handle_mode_transition_failure();
                }
            }
            if self.mode_helpers.transition_state == TransitionState::Done {
                self.handle_mode_reached();
            }
        }
    }

    fn handle_mode_transition_failure(&mut self) {
        self.mode_helpers.finish(false);
        if let Some(requestor) = self.mode_helpers.tc_id {
            self.send_telemetry(
                Some(requestor),
                mgm::response::Response::ModeFailure(ModeFailure::Timeout),
            );
        }
        self.mode_leaf_helper
            .report_tx
            .send(ModeReport::SetModeTimeout)
            .unwrap();
    }

    fn handle_mode_reached(&mut self) {
        self.mode_helpers.finish(true);
        log::info!(
            "{} announcing mode: {:?}",
            self.id.str(),
            self.mode_helpers.current
        );
        if let Some(requestor) = self.mode_helpers.tc_id {
            self.send_mode_tm(requestor);
        }
        // Inform our parent about mode changes.
        self.report_mode_to_parent();
    }

    fn report_mode_to_parent(&self) {
        self.mode_leaf_helper
            .report_tx
            .send(ModeReport::Mode(self.mode_helpers.current))
            .unwrap();
    }

    fn send_mode_tm(&self, requestor: CcsdsPacketIdAndPsc) {
        self.send_telemetry(Some(requestor), mgm::response::Response::Ok);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        mpsc::{self, TryRecvError},
        Arc,
    };

    use arbitrary_int::u11;
    use models::{
        mgm::request::HkRequest,
        pcdu::{SwitchRequest, SwitchState, SwitchStateBinary},
        Apid, ComponentId, TcHeader,
    };
    use satrs::{request::GenericMessage, spacepackets::SpacePacketHeader};
    use satrs_minisim::acs::lis3mdl::MgmLis3RawValues;

    use crate::eps::pcdu::{SharedSwitchSet, SwitchMap, SwitchSet};

    use super::*;

    #[derive(Debug, Copy, Clone)]
    pub enum MgmSelect {
        _0,
        _1,
    }

    impl MgmSelect {
        pub fn id(&self) -> ComponentId {
            match self {
                MgmSelect::_0 => ComponentId::AcsMgm0,
                MgmSelect::_1 => ComponentId::AcsMgm1,
            }
        }
    }

    pub fn create_request_tc(
        select: MgmSelect,
        request: models::mgm::request::Request,
    ) -> models::ccsds::CcsdsTcPacketOwned {
        models::ccsds::CcsdsTcPacketOwned::new_with_request(
            SpacePacketHeader::new_from_apid(u11::new(Apid::Acs as u16)),
            TcHeader::new(select.id(), models::MessageType::Ping),
            request,
        )
    }

    #[derive(Default)]
    pub struct TestSpiInterface {
        pub call_count: u32,
        pub next_mgm_data: MgmLis3RawValues,
    }

    impl SpiCommunication for TestSpiInterface {
        type Error = ();

        fn transfer(&mut self, _tx: &[u8], rx: &mut [u8]) -> Result<(), Self::Error> {
            rx[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2]
                .copy_from_slice(&self.next_mgm_data.x.to_le_bytes());
            rx[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2]
                .copy_from_slice(&self.next_mgm_data.y.to_le_bytes());
            rx[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2]
                .copy_from_slice(&self.next_mgm_data.z.to_le_bytes());
            self.call_count += 1;
            Ok(())
        }
    }

    #[allow(dead_code)]
    pub struct MgmTestbench {
        pub assembly_mode_request_tx: mpsc::SyncSender<ModeRequest>,
        pub mode_report_rx: mpsc::Receiver<ModeReport>,
        pub shared_switch_set: SharedSwitchSet,
        pub tc_tx: mpsc::SyncSender<CcsdsTcPacketOwned>,
        pub tm_rx: mpsc::Receiver<CcsdsTmPacketOwned>,
        pub switch_rx: mpsc::Receiver<GenericMessage<SwitchRequest>>,
        pub handler: MgmHandlerLis3Mdl<TestSpiInterface>,
    }

    impl MgmTestbench {
        pub fn new() -> Self {
            let (assembly_mode_request_tx, assembly_mode_request_rx) = mpsc::sync_channel(5);
            let (mode_report_tx, mode_report_rx) = mpsc::sync_channel(5);
            let mode_leaf_helper = ModeLeafHelper {
                request_rx: assembly_mode_request_rx,
                report_tx: mode_report_tx,
            };
            let (tc_tx, tc_rx) = mpsc::sync_channel(10);
            let (tm_tx, tm_rx) = mpsc::sync_channel(10);
            let (switcher_tx, switch_rx) = mpsc::sync_channel(10);
            let shared_mgm_set = Arc::default();
            let mut switch_map = SwitchMap::new();
            switch_map.insert(SwitchId::Mgm0, SwitchState::Off);
            let switch_map = SwitchSet::new(switch_map);
            let shared_switch_set = SharedSwitchSet::new(Mutex::new(switch_map));
            let handler = MgmHandlerLis3Mdl::new(
                MgmId::_0,
                tc_rx,
                tm_tx,
                PowerSwitchHelper::new(switcher_tx, shared_switch_set.clone()),
                TestSpiInterface::default(),
                shared_mgm_set,
                mode_leaf_helper,
            );
            Self {
                assembly_mode_request_tx,
                mode_report_rx,
                shared_switch_set,
                switch_rx,
                handler,
                tm_rx,
                tc_tx,
            }
        }
    }

    #[test]
    fn test_basic_handler() {
        let mut testbench = MgmTestbench::new();
        assert_eq!(testbench.handler.com_interface.call_count, 0);
        assert_eq!(testbench.handler.mode(), DeviceMode::Off);
        testbench.handler.periodic_operation();
        // Handler is OFF, no changes expected.
        assert_eq!(testbench.handler.com_interface.call_count, 0);
        assert_eq!(testbench.handler.mode(), DeviceMode::Off);
    }

    #[test]
    fn test_normal_handler() {
        let mut testbench = MgmTestbench::new();
        testbench
            .tc_tx
            .send(create_request_tc(
                MgmSelect::_0,
                mgm::request::Request::Mode(DeviceMode::Normal),
            ))
            .unwrap();
        testbench.handler.periodic_operation();
        assert_eq!(testbench.handler.mode(), DeviceMode::Off);

        // Verify power switch handling.
        let switch_req = testbench.switch_rx.try_recv().expect("no switch request");
        assert_eq!(switch_req.message.switch_id, SwitchId::Mgm0);
        assert_eq!(switch_req.message.target_state, SwitchStateBinary::On);

        // This simulates one cycle for the power switch to update.
        testbench
            .shared_switch_set
            .lock()
            .unwrap()
            .set_switch_state(SwitchId::Mgm0, SwitchState::On);

        // Now the power switch is updated and the mode request should be completed.
        testbench.handler.periodic_operation();

        assert_eq!(testbench.handler.mode(), DeviceMode::Normal);

        let tm_packet = testbench.tm_rx.try_recv().expect("no mode reply generated");

        assert_eq!(tm_packet.tm_header.sender_id, ComponentId::AcsMgm0);

        let response = postcard::from_bytes::<models::mgm::response::Response>(&tm_packet.payload)
            .expect("failed to deserialize mode reply");
        matches!(response, models::mgm::response::Response::Ok);
        // The device should have been polled once.
        assert_eq!(testbench.handler.com_interface.call_count, 1);
        let mgm_set = *testbench.handler.shared_mgm_set.lock().unwrap();
        assert!(mgm_set.x < 0.001);
        assert!(mgm_set.y < 0.001);
        assert!(mgm_set.z < 0.001);
        assert!(mgm_set.valid);

        matches!(testbench.tm_rx.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn test_normal_handler_mgm_set_conversion() {
        let mut testbench = MgmTestbench::new();
        let raw_values = MgmLis3RawValues {
            x: 1000,
            y: -1000,
            z: 1000,
        };
        testbench.handler.com_interface.next_mgm_data = raw_values;
        testbench
            .tc_tx
            .send(create_request_tc(
                MgmSelect::_0,
                mgm::request::Request::Mode(DeviceMode::Normal),
            ))
            .unwrap();
        testbench.handler.periodic_operation();

        // This simulates one cycle for the power switch to update.
        testbench
            .shared_switch_set
            .lock()
            .unwrap()
            .set_switch_state(SwitchId::Mgm0, SwitchState::On);

        // Now the power switch is updated and the mode request should be completed.
        testbench.handler.periodic_operation();

        let mgm_set = *testbench.handler.shared_mgm_set.lock().unwrap();
        let expected_x =
            raw_values.x as f32 * GAUSS_TO_MICROTESLA_FACTOR as f32 * FIELD_LSB_PER_GAUSS_4_SENS;
        let expected_y =
            raw_values.y as f32 * GAUSS_TO_MICROTESLA_FACTOR as f32 * FIELD_LSB_PER_GAUSS_4_SENS;
        let expected_z =
            raw_values.z as f32 * GAUSS_TO_MICROTESLA_FACTOR as f32 * FIELD_LSB_PER_GAUSS_4_SENS;
        let x_diff = (mgm_set.x - expected_x).abs();
        let y_diff = (mgm_set.y - expected_y).abs();
        let z_diff = (mgm_set.z - expected_z).abs();
        assert!(x_diff < 0.001, "x diff too large: {}", x_diff);
        assert!(y_diff < 0.001, "y diff too large: {}", y_diff);
        assert!(z_diff < 0.001, "z diff too large: {}", z_diff);
        assert!(mgm_set.valid);
    }

    #[test]
    fn test_hk_one_shot_device_off() {
        let mut testbench = MgmTestbench::new();
        // Device handler is initially off, first set will be invalid.
        testbench
            .tc_tx
            .send(create_request_tc(
                MgmSelect::_0,
                mgm::request::Request::Hk(HkRequest {
                    id: mgm::request::HkId::Sensor,
                    req_type: HkRequestType::OneShot,
                }),
            ))
            .unwrap();
        testbench.handler.periodic_operation();

        // This simulates one cycle for the power switch to update.
        testbench
            .shared_switch_set
            .lock()
            .unwrap()
            .set_switch_state(SwitchId::Mgm0, SwitchState::On);

        // Now the power switch is updated and the mode request should be completed.
        testbench.handler.periodic_operation();

        let tm_packet = testbench.tm_rx.try_recv().expect("no mode reply generated");

        assert_eq!(tm_packet.tm_header.sender_id, ComponentId::AcsMgm0);

        let response = postcard::from_bytes::<models::mgm::response::Response>(&tm_packet.payload)
            .expect("failed to deserialize mode reply");
        if let models::mgm::response::Response::Hk(mgm::response::HkResponse::MgmData(data)) =
            response
        {
            assert_eq!(data.valid, false);
            assert!(data.x < 0.001);
            assert!(data.y < 0.001);
            assert!(data.z < 0.001);
        } else {
            panic!("expected hk response");
        }

        matches!(testbench.tm_rx.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn test_hk_device_normal() {
        let mut testbench = MgmTestbench::new();
        testbench
            .tc_tx
            .send(create_request_tc(
                MgmSelect::_0,
                mgm::request::Request::Mode(DeviceMode::Normal),
            ))
            .unwrap();
        // This simulates one cycle for the power switch to update.
        testbench
            .shared_switch_set
            .lock()
            .unwrap()
            .set_switch_state(SwitchId::Mgm0, SwitchState::On);
        testbench.handler.periodic_operation();
        assert_eq!(testbench.handler.mode(), DeviceMode::Normal);

        testbench
            .tc_tx
            .send(create_request_tc(
                MgmSelect::_0,
                mgm::request::Request::Hk(HkRequest {
                    id: mgm::request::HkId::Sensor,
                    req_type: HkRequestType::OneShot,
                }),
            ))
            .unwrap();
        testbench.handler.periodic_operation();

        let mode_tm = testbench.tm_rx.try_recv().expect("no mode reply generated");

        assert_eq!(mode_tm.tm_header.sender_id, ComponentId::AcsMgm0);

        let response = postcard::from_bytes::<models::mgm::response::Response>(&mode_tm.payload)
            .expect("failed to deserialize mode reply");
        matches!(response, models::mgm::response::Response::Ok);

        let hk_tm = testbench.tm_rx.try_recv().expect("no hk reply generated");

        assert_eq!(hk_tm.tm_header.sender_id, ComponentId::AcsMgm0);

        let response = postcard::from_bytes::<models::mgm::response::Response>(&hk_tm.payload)
            .expect("failed to deserialize mode reply");
        if let models::mgm::response::Response::Hk(mgm::response::HkResponse::MgmData(data)) =
            response
        {
            // Set is now valid.
            assert_eq!(data.valid, true);
            assert!(data.x < 0.001);
            assert!(data.y < 0.001);
            assert!(data.z < 0.001);
        } else {
            panic!("expected hk response");
        }

        matches!(testbench.tm_rx.try_recv(), Err(TryRecvError::Empty));
    }
}
