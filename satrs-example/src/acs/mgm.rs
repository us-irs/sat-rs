use derive_new::new;
use satrs::hk::{HkRequest, HkRequestVariant};
use satrs::mode_tree::{ModeChild, ModeNode};
use satrs::power::{PowerSwitchInfo, PowerSwitcherCommandSender};
use satrs_example::ids::generic_pus::PUS_MODE;
use satrs_example::{DeviceMode, TimestampHelper};
use satrs_minisim::acs::lis3mdl::{
    MgmLis3MdlReply, MgmLis3RawValues, FIELD_LSB_PER_GAUSS_4_SENS, GAUSS_TO_MICROTESLA_FACTOR,
};
use satrs_minisim::acs::MgmRequestLis3Mdl;
use satrs_minisim::eps::PcduSwitch;
use satrs_minisim::{SerializableSimMsgPayload, SimReply, SimRequest};
use std::fmt::Debug;
use std::sync::mpsc::{self};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use satrs::mode::{
    ModeAndSubmode, ModeError, ModeProvider, ModeReply, ModeRequestHandler,
    ModeRequestHandlerMpscBounded,
};
use satrs::pus::{EcssTmSender, PusTmVariant};
use satrs::request::{GenericMessage, MessageMetadata, UniqueApidTargetId};
use satrs_example::config::components::NO_SENDER;

use crate::hk::PusHkHelper;
use crate::pus::hk::{HkReply, HkReplyVariant};
use crate::requests::CompositeRequest;
use crate::spi::SpiInterface;
use crate::tmtc::sender::TmTcSender;

use serde::{Deserialize, Serialize};

pub const NR_OF_DATA_AND_CFG_REGISTERS: usize = 14;

// Register adresses to access various bytes from the raw reply.
pub const X_LOWBYTE_IDX: usize = 9;
pub const Y_LOWBYTE_IDX: usize = 11;
pub const Z_LOWBYTE_IDX: usize = 13;

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[repr(u32)]
pub enum SetId {
    SensorData = 0,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub enum TransitionState {
    #[default]
    Idle,
    PowerSwitching,
    Done,
}

#[derive(Default)]
pub struct SpiDummyInterface {
    pub dummy_values: MgmLis3RawValues,
}

impl SpiInterface for SpiDummyInterface {
    type Error = ();

    fn transfer(&mut self, _tx: &[u8], rx: &mut [u8]) -> Result<(), Self::Error> {
        rx[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2].copy_from_slice(&self.dummy_values.x.to_le_bytes());
        rx[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2].copy_from_slice(&self.dummy_values.y.to_be_bytes());
        rx[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2].copy_from_slice(&self.dummy_values.z.to_be_bytes());
        Ok(())
    }
}

pub struct SpiSimInterface {
    pub sim_request_tx: mpsc::Sender<SimRequest>,
    pub sim_reply_rx: mpsc::Receiver<SimReply>,
}

impl SpiInterface for SpiSimInterface {
    type Error = ();

    // Right now, we only support requesting sensor data and not configuration of the sensor.
    fn transfer(&mut self, _tx: &[u8], rx: &mut [u8]) -> Result<(), Self::Error> {
        let mgm_sensor_request = MgmRequestLis3Mdl::RequestSensorData;
        if let Err(e) = self
            .sim_request_tx
            .send(SimRequest::new_with_epoch_time(mgm_sensor_request))
        {
            log::error!("failed to send MGM LIS3 request: {}", e);
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
                log::warn!("MGM LIS3 SIM reply timeout: {}", e);
            }
        }
        Ok(())
    }
}

pub enum SpiSimInterfaceWrapper {
    Dummy(SpiDummyInterface),
    Sim(SpiSimInterface),
}

impl SpiInterface for SpiSimInterfaceWrapper {
    type Error = ();

    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<(), Self::Error> {
        match self {
            SpiSimInterfaceWrapper::Dummy(dummy) => dummy.transfer(tx, rx),
            SpiSimInterfaceWrapper::Sim(sim_if) => sim_if.transfer(tx, rx),
        }
    }
}

#[derive(Default, Debug, Copy, Clone, Serialize, Deserialize)]
pub struct MgmData {
    pub valid: bool,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Default)]
pub struct BufWrapper {
    tx_buf: [u8; 32],
    rx_buf: [u8; 32],
    tm_buf: [u8; 32],
}

pub struct ModeHelpers {
    current: ModeAndSubmode,
    target: Option<ModeAndSubmode>,
    requestor_info: Option<MessageMetadata>,
    transition_state: TransitionState,
}

impl Default for ModeHelpers {
    fn default() -> Self {
        Self {
            current: ModeAndSubmode::new(DeviceMode::Off as u32, 0),
            target: Default::default(),
            requestor_info: Default::default(),
            transition_state: Default::default(),
        }
    }
}

/// Example MGM device handler strongly based on the LIS3MDL MEMS device.
#[derive(new)]
#[allow(clippy::too_many_arguments)]
pub struct MgmHandlerLis3Mdl<
    ComInterface: SpiInterface,
    SwitchHelper: PowerSwitchInfo<PcduSwitch> + PowerSwitcherCommandSender<PcduSwitch>,
> {
    id: UniqueApidTargetId,
    dev_str: &'static str,
    mode_node: ModeRequestHandlerMpscBounded,
    composite_request_rx: mpsc::Receiver<GenericMessage<CompositeRequest>>,
    hk_reply_tx: mpsc::SyncSender<GenericMessage<HkReply>>,
    switch_helper: SwitchHelper,
    tm_sender: TmTcSender,
    pub com_interface: ComInterface,
    shared_mgm_set: Arc<Mutex<MgmData>>,
    #[new(value = "PusHkHelper::new(id)")]
    hk_helper: PusHkHelper,
    #[new(default)]
    mode_helpers: ModeHelpers,
    #[new(default)]
    bufs: BufWrapper,
    #[new(default)]
    stamp_helper: TimestampHelper,
}

impl<
        ComInterface: SpiInterface,
        SwitchHelper: PowerSwitchInfo<PcduSwitch> + PowerSwitcherCommandSender<PcduSwitch>,
    > MgmHandlerLis3Mdl<ComInterface, SwitchHelper>
{
    pub fn periodic_operation(&mut self) {
        self.stamp_helper.update_from_now();
        // Handle requests.
        self.handle_composite_requests();
        self.handle_mode_requests();
        if let Some(target_mode_submode) = self.mode_helpers.target {
            self.handle_mode_transition(target_mode_submode);
        }
        if self.mode() == DeviceMode::Normal as u32 {
            log::trace!("polling LIS3MDL sensor {}", self.dev_str);
            self.poll_sensor();
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
                let mgm_snapshot = *self.shared_mgm_set.lock().unwrap();
                if let Ok(hk_tm) = self.hk_helper.generate_hk_report_packet(
                    self.stamp_helper.stamp(),
                    SetId::SensorData as u32,
                    &mut |hk_buf| {
                        hk_buf[0] = mgm_snapshot.valid as u8;
                        hk_buf[1..5].copy_from_slice(&mgm_snapshot.x.to_be_bytes());
                        hk_buf[5..9].copy_from_slice(&mgm_snapshot.y.to_be_bytes());
                        hk_buf[9..13].copy_from_slice(&mgm_snapshot.z.to_be_bytes());
                        Ok(13)
                    },
                    &mut self.bufs.tm_buf,
                ) {
                    // TODO: If sending the TM fails, we should also send a failure reply.
                    self.tm_sender
                        .send_tm(self.id.id(), PusTmVariant::Direct(hk_tm))
                        .expect("failed to send HK TM");
                    self.hk_reply_tx
                        .send(GenericMessage::new(
                            *requestor_info,
                            HkReply::new(hk_request.unique_id, HkReplyVariant::Ack),
                        ))
                        .expect("failed to send HK reply");
                } else {
                    // TODO: Send back failure reply. Need result code for this.
                    log::error!("TM buffer too small to generate HK data");
                }
            }
            HkRequestVariant::EnablePeriodic => todo!(),
            HkRequestVariant::DisablePeriodic => todo!(),
            HkRequestVariant::ModifyCollectionInterval(_) => todo!(),
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
                    satrs::queue::GenericReceiveError::Empty => break,
                    satrs::queue::GenericReceiveError::TxDisconnected(e) => {
                        log::warn!("{}: failed to receive mode request: {:?}", self.dev_str, e);
                    }
                },
            }
        }
    }

    pub fn poll_sensor(&mut self) {
        // Communicate with the device. This is actually how to read the data from the LIS3 device
        // SPI interface.
        self.com_interface
            .transfer(
                &self.bufs.tx_buf[0..NR_OF_DATA_AND_CFG_REGISTERS + 1],
                &mut self.bufs.rx_buf[0..NR_OF_DATA_AND_CFG_REGISTERS + 1],
            )
            .expect("failed to transfer data");
        let x_raw = i16::from_le_bytes(
            self.bufs.rx_buf[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2]
                .try_into()
                .unwrap(),
        );
        let y_raw = i16::from_le_bytes(
            self.bufs.rx_buf[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2]
                .try_into()
                .unwrap(),
        );
        let z_raw = i16::from_le_bytes(
            self.bufs.rx_buf[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2]
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

    pub fn handle_mode_transition(&mut self, target_mode_submode: ModeAndSubmode) {
        if target_mode_submode.mode() == DeviceMode::On as u32
            || target_mode_submode.mode() == DeviceMode::Normal as u32
        {
            if self.mode_helpers.transition_state == TransitionState::Idle {
                let result = self
                    .switch_helper
                    .send_switch_on_cmd(MessageMetadata::new(0, self.id.id()), PcduSwitch::Mgm);
                if result.is_err() {
                    // Could not send switch command.. still continue with transition.
                    log::error!("failed to send switch on command");
                }
                self.mode_helpers.transition_state = TransitionState::PowerSwitching;
            }
            if self.mode_helpers.transition_state == TransitionState::PowerSwitching
                && self
                    .switch_helper
                    .is_switch_on(PcduSwitch::Mgm)
                    .expect("switch info error")
            {
                self.mode_helpers.transition_state = TransitionState::Done;
            }
            if self.mode_helpers.transition_state == TransitionState::Done {
                self.mode_helpers.current = self.mode_helpers.target.unwrap();
                self.handle_mode_reached(self.mode_helpers.requestor_info)
                    .expect("failed to handle mode reached");
                self.mode_helpers.transition_state = TransitionState::Idle;
            }
        }
    }
}

impl<
        ComInterface: SpiInterface,
        SwitchHelper: PowerSwitchInfo<PcduSwitch> + PowerSwitcherCommandSender<PcduSwitch>,
    > ModeProvider for MgmHandlerLis3Mdl<ComInterface, SwitchHelper>
{
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_helpers.current
    }
}

impl<
        ComInterface: SpiInterface,
        SwitchHelper: PowerSwitchInfo<PcduSwitch> + PowerSwitcherCommandSender<PcduSwitch>,
    > ModeRequestHandler for MgmHandlerLis3Mdl<ComInterface, SwitchHelper>
{
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
        self.mode_helpers.current = mode_and_submode;
        if mode_and_submode.mode() == DeviceMode::Off as u32 {
            self.shared_mgm_set.lock().unwrap().valid = false;
            self.handle_mode_reached(Some(requestor))?;
        } else if mode_and_submode.mode() == DeviceMode::Normal as u32
            || mode_and_submode.mode() == DeviceMode::On as u32
        {
            // TODO: Write helper method for the struct? Might help for other handlers as well..
            self.mode_helpers.transition_state = TransitionState::Idle;
            self.mode_helpers.requestor_info = Some(requestor);
            self.mode_helpers.target = Some(mode_and_submode);
        }
        Ok(())
    }

    fn announce_mode(&self, _requestor_info: Option<MessageMetadata>, _recursive: bool) {
        log::info!(
            "{} announcing mode: {:?}",
            self.dev_str,
            self.mode_and_submode()
        );
    }

    fn handle_mode_reached(
        &mut self,
        requestor: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        self.mode_helpers.target = None;
        self.announce_mode(requestor, false);
        if let Some(requestor) = requestor {
            if requestor.sender_id() == NO_SENDER {
                return Ok(());
            }
            if requestor.sender_id() != PUS_MODE.id() {
                log::warn!(
                    "can not send back mode reply to sender {:x}",
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
        if requestor.sender_id() != PUS_MODE.id() {
            log::warn!(
                "can not send back mode reply to sender {}",
                requestor.sender_id()
            );
        }
        self.mode_node
            .send_mode_reply(requestor, reply)
            .map_err(ModeError::Send)?;
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

impl<
        ComInterface: SpiInterface,
        SwitchHelper: PowerSwitchInfo<PcduSwitch> + PowerSwitcherCommandSender<PcduSwitch>,
    > ModeNode for MgmHandlerLis3Mdl<ComInterface, SwitchHelper>
{
    fn id(&self) -> satrs::ComponentId {
        self.id.into()
    }
}

impl<
        ComInterface: SpiInterface,
        SwitchHelper: PowerSwitchInfo<PcduSwitch> + PowerSwitcherCommandSender<PcduSwitch>,
    > ModeChild for MgmHandlerLis3Mdl<ComInterface, SwitchHelper>
{
    type Sender = mpsc::SyncSender<GenericMessage<ModeReply>>;

    fn add_mode_parent(&mut self, id: satrs::ComponentId, reply_sender: Self::Sender) {
        self.mode_node.add_message_target(id, reply_sender);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{mpsc, Arc},
    };

    use satrs::{
        mode::{ModeReply, ModeRequest},
        mode_tree::ModeParent,
        power::SwitchStateBinary,
        request::{GenericMessage, UniqueApidTargetId},
        tmtc::PacketAsVec,
        ComponentId,
    };
    use satrs_example::ids::{acs::ASSEMBLY, Apid};
    use satrs_minisim::acs::lis3mdl::MgmLis3RawValues;

    use crate::{eps::TestSwitchHelper, pus::hk::HkReply, requests::CompositeRequest};

    use super::*;

    #[derive(Default)]
    pub struct TestSpiInterface {
        pub call_count: u32,
        pub next_mgm_data: MgmLis3RawValues,
    }

    impl SpiInterface for TestSpiInterface {
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
        pub mode_request_tx: mpsc::SyncSender<GenericMessage<ModeRequest>>,
        pub mode_reply_rx_to_pus: mpsc::Receiver<GenericMessage<ModeReply>>,
        pub mode_reply_rx_to_parent: mpsc::Receiver<GenericMessage<ModeReply>>,
        pub composite_request_tx: mpsc::Sender<GenericMessage<CompositeRequest>>,
        pub hk_reply_rx: mpsc::Receiver<GenericMessage<HkReply>>,
        pub tm_rx: mpsc::Receiver<PacketAsVec>,
        pub handler: MgmHandlerLis3Mdl<TestSpiInterface, TestSwitchHelper>,
    }

    #[derive(Default)]
    pub struct MgmAssemblyMock(
        pub HashMap<ComponentId, mpsc::SyncSender<GenericMessage<ModeRequest>>>,
    );

    impl ModeNode for MgmAssemblyMock {
        fn id(&self) -> satrs::ComponentId {
            PUS_MODE.into()
        }
    }

    impl ModeParent for MgmAssemblyMock {
        type Sender = mpsc::SyncSender<GenericMessage<ModeRequest>>;

        fn add_mode_child(&mut self, id: satrs::ComponentId, request_sender: Self::Sender) {
            self.0.insert(id, request_sender);
        }
    }

    #[derive(Default)]
    pub struct PusMock {
        pub request_sender_map: HashMap<ComponentId, mpsc::SyncSender<GenericMessage<ModeRequest>>>,
    }

    impl ModeNode for PusMock {
        fn id(&self) -> satrs::ComponentId {
            PUS_MODE.into()
        }
    }

    impl ModeParent for PusMock {
        type Sender = mpsc::SyncSender<GenericMessage<ModeRequest>>;

        fn add_mode_child(&mut self, id: satrs::ComponentId, request_sender: Self::Sender) {
            self.request_sender_map.insert(id, request_sender);
        }
    }

    impl MgmTestbench {
        pub fn new() -> Self {
            let (request_tx, request_rx) = mpsc::sync_channel(5);
            let (reply_tx_to_pus, reply_rx_to_pus) = mpsc::sync_channel(5);
            let (reply_tx_to_parent, reply_rx_to_parent) = mpsc::sync_channel(5);
            let id = UniqueApidTargetId::new(Apid::Acs as u16, 1);
            let mode_node = ModeRequestHandlerMpscBounded::new(id.into(), request_rx);
            let (composite_request_tx, composite_request_rx) = mpsc::channel();
            let (hk_reply_tx, hk_reply_rx) = mpsc::sync_channel(10);
            let (tm_tx, tm_rx) = mpsc::sync_channel(10);
            let tm_sender = TmTcSender::Heap(tm_tx);
            let shared_mgm_set = Arc::default();
            let mut handler = MgmHandlerLis3Mdl::new(
                id,
                "TEST_MGM",
                mode_node,
                composite_request_rx,
                hk_reply_tx,
                TestSwitchHelper::default(),
                tm_sender,
                TestSpiInterface::default(),
                shared_mgm_set,
            );
            handler.add_mode_parent(PUS_MODE.into(), reply_tx_to_pus);
            handler.add_mode_parent(ASSEMBLY.into(), reply_tx_to_parent);
            Self {
                mode_request_tx: request_tx,
                mode_reply_rx_to_pus: reply_rx_to_pus,
                mode_reply_rx_to_parent: reply_rx_to_parent,
                composite_request_tx,
                handler,
                tm_rx,
                hk_reply_rx,
            }
        }
    }

    #[test]
    fn test_basic_handler() {
        let mut testbench = MgmTestbench::new();
        assert_eq!(testbench.handler.com_interface.call_count, 0);
        assert_eq!(
            testbench.handler.mode_and_submode().mode(),
            DeviceMode::Off as u32
        );
        assert_eq!(testbench.handler.mode_and_submode().submode(), 0_u16);
        testbench.handler.periodic_operation();
        // Handler is OFF, no changes expected.
        assert_eq!(testbench.handler.com_interface.call_count, 0);
        assert_eq!(
            testbench.handler.mode_and_submode().mode(),
            DeviceMode::Off as u32
        );
        assert_eq!(testbench.handler.mode_and_submode().submode(), 0_u16);
    }

    #[test]
    fn test_normal_handler() {
        let mut testbench = MgmTestbench::new();
        testbench
            .mode_request_tx
            .send(GenericMessage::new(
                MessageMetadata::new(0, PUS_MODE.id()),
                ModeRequest::SetMode {
                    mode_and_submode: ModeAndSubmode::new(DeviceMode::Normal as u32, 0),
                    forced: false,
                },
            ))
            .expect("failed to send mode request");
        testbench.handler.periodic_operation();
        assert_eq!(
            testbench.handler.mode_and_submode().mode(),
            DeviceMode::Normal as u32
        );
        assert_eq!(testbench.handler.mode_and_submode().submode(), 0);

        // Verify power switch handling.
        let mut switch_requests = testbench.handler.switch_helper.switch_requests.borrow_mut();
        assert_eq!(switch_requests.len(), 1);
        let switch_req = switch_requests.pop_front().expect("no switch request");
        assert_eq!(switch_req.target_state, SwitchStateBinary::On);
        assert_eq!(switch_req.switch_id, PcduSwitch::Mgm);
        let mut switch_info_requests = testbench
            .handler
            .switch_helper
            .switch_info_requests
            .borrow_mut();
        assert_eq!(switch_info_requests.len(), 1);
        let switch_info_req = switch_info_requests.pop_front().expect("no switch request");
        assert_eq!(switch_info_req, PcduSwitch::Mgm);

        let mode_reply = testbench
            .mode_reply_rx_to_pus
            .try_recv()
            .expect("no mode reply generated");
        match mode_reply.message {
            ModeReply::ModeReply(mode) => {
                assert_eq!(mode.mode(), DeviceMode::Normal as u32);
                assert_eq!(mode.submode(), 0);
            }
            _ => panic!("unexpected mode reply"),
        }
        // The device should have been polled once.
        assert_eq!(testbench.handler.com_interface.call_count, 1);
        let mgm_set = *testbench.handler.shared_mgm_set.lock().unwrap();
        assert!(mgm_set.x < 0.001);
        assert!(mgm_set.y < 0.001);
        assert!(mgm_set.z < 0.001);
        assert!(mgm_set.valid);
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
            .mode_request_tx
            .send(GenericMessage::new(
                MessageMetadata::new(0, PUS_MODE.id()),
                ModeRequest::SetMode {
                    mode_and_submode: ModeAndSubmode::new(DeviceMode::Normal as u32, 0),
                    forced: false,
                },
            ))
            .expect("failed to send mode request");
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
}
