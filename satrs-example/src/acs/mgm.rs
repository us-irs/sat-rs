use derive_new::new;
use satrs::hk::{HkRequest, HkRequestVariant};
use satrs::queue::{GenericSendError, GenericTargetedMessagingError};
use satrs::spacepackets::ecss::hk;
use satrs::spacepackets::ecss::tm::{PusTmCreator, PusTmSecondaryHeader};
use satrs::spacepackets::SpHeader;
use satrs_example::{DeviceMode, TimestampHelper};
use satrs_minisim::acs::lis3mdl::{
    MgmLis3MdlReply, FIELD_LSB_PER_GAUSS_4_SENS, GAUSS_TO_MICROTESLA_FACTOR,
};
use satrs_minisim::acs::MgmRequestLis3Mdl;
use satrs_minisim::{SerializableSimMsgPayload, SimReply, SimRequest};
use std::sync::mpsc::{self};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use satrs::mode::{
    ModeAndSubmode, ModeError, ModeProvider, ModeReply, ModeRequest, ModeRequestHandler,
};
use satrs::pus::{EcssTmSender, PusTmVariant};
use satrs::request::{GenericMessage, MessageMetadata, UniqueApidTargetId};
use satrs_example::config::components::PUS_MODE_SERVICE;

use crate::pus::hk::{HkReply, HkReplyVariant};
use crate::requests::CompositeRequest;

use serde::{Deserialize, Serialize};

pub const NR_OF_DATA_AND_CFG_REGISTERS: usize = 14;

// Register adresses to access various bytes from the raw reply.
pub const X_LOWBYTE_IDX: usize = 9;
pub const Y_LOWBYTE_IDX: usize = 11;
pub const Z_LOWBYTE_IDX: usize = 13;

pub trait SpiInterface {
    type Error;
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<(), Self::Error>;
}

#[derive(Default)]
pub struct SpiDummyInterface {
    pub dummy_val_0: i16,
    pub dummy_val_1: i16,
    pub dummy_val_2: i16,
}

impl SpiInterface for SpiDummyInterface {
    type Error = ();

    fn transfer(&mut self, _tx: &[u8], rx: &mut [u8]) -> Result<(), Self::Error> {
        rx[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2].copy_from_slice(&self.dummy_val_0.to_le_bytes());
        rx[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2].copy_from_slice(&self.dummy_val_1.to_be_bytes());
        rx[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2].copy_from_slice(&self.dummy_val_2.to_be_bytes());
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
        self.sim_request_tx
            .send(SimRequest::new_with_epoch_time(mgm_sensor_request))
            .expect("failed to send request");
        let sim_reply = self
            .sim_reply_rx
            .recv_timeout(Duration::from_millis(100))
            .expect("reply timeout");
        let sim_reply_lis3 =
            MgmLis3MdlReply::from_sim_message(&sim_reply).expect("failed to parse LIS3 reply");
        rx[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2].copy_from_slice(&sim_reply_lis3.raw.x.to_le_bytes());
        rx[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2].copy_from_slice(&sim_reply_lis3.raw.y.to_be_bytes());
        rx[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2].copy_from_slice(&sim_reply_lis3.raw.z.to_be_bytes());
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

pub struct MpscModeLeafInterface {
    pub request_rx: mpsc::Receiver<GenericMessage<ModeRequest>>,
    pub reply_to_pus_tx: mpsc::Sender<GenericMessage<ModeReply>>,
    pub reply_to_parent_tx: mpsc::Sender<GenericMessage<ModeReply>>,
}

/// Example MGM device handler strongly based on the LIS3MDL MEMS device.
#[derive(new)]
#[allow(clippy::too_many_arguments)]
pub struct MgmHandlerLis3Mdl<ComInterface: SpiInterface, TmSender: EcssTmSender> {
    id: UniqueApidTargetId,
    dev_str: &'static str,
    mode_interface: MpscModeLeafInterface,
    composite_request_rx: mpsc::Receiver<GenericMessage<CompositeRequest>>,
    hk_reply_tx: mpsc::Sender<GenericMessage<HkReply>>,
    tm_sender: TmSender,
    com_interface: ComInterface,
    shared_mgm_set: Arc<Mutex<MgmData>>,
    #[new(value = "ModeAndSubmode::new(satrs_example::DeviceMode::Off as u32, 0)")]
    mode_and_submode: ModeAndSubmode,
    #[new(default)]
    tx_buf: [u8; 32],
    #[new(default)]
    rx_buf: [u8; 32],
    #[new(default)]
    tm_buf: [u8; 16],
    #[new(default)]
    stamp_helper: TimestampHelper,
}

impl<ComInterface: SpiInterface, TmSender: EcssTmSender> MgmHandlerLis3Mdl<ComInterface, TmSender> {
    pub fn periodic_operation(&mut self) {
        self.stamp_helper.update_from_now();
        // Handle requests.
        self.handle_composite_requests();
        self.handle_mode_requests();
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
                self.hk_reply_tx
                    .send(GenericMessage::new(
                        *requestor_info,
                        HkReply::new(hk_request.unique_id, HkReplyVariant::Ack),
                    ))
                    .expect("failed to send HK reply");
                let sec_header = PusTmSecondaryHeader::new(
                    3,
                    hk::Subservice::TmHkPacket as u8,
                    0,
                    0,
                    self.stamp_helper.stamp(),
                );
                let mgm_snapshot = *self.shared_mgm_set.lock().unwrap();
                // Use binary serialization here. We want the data to be tightly packed.
                self.tm_buf[0] = mgm_snapshot.valid as u8;
                self.tm_buf[1..5].copy_from_slice(&mgm_snapshot.x.to_be_bytes());
                self.tm_buf[5..9].copy_from_slice(&mgm_snapshot.y.to_be_bytes());
                self.tm_buf[9..13].copy_from_slice(&mgm_snapshot.z.to_be_bytes());
                let hk_tm = PusTmCreator::new(
                    SpHeader::new_from_apid(self.id.apid),
                    sec_header,
                    &self.tm_buf[0..12],
                    true,
                );
                self.tm_sender
                    .send_tm(self.id.id(), PusTmVariant::Direct(hk_tm))
                    .expect("failed to send HK TM");
            }
            HkRequestVariant::EnablePeriodic => todo!(),
            HkRequestVariant::DisablePeriodic => todo!(),
            HkRequestVariant::ModifyCollectionInterval(_) => todo!(),
        }
    }

    pub fn poll_sensor(&mut self) {
        // Communicate with the device. This is actually how to read the data from the LIS3 device
        // SPI interface.
        let result = self.com_interface.transfer(
            &self.tx_buf[0..NR_OF_DATA_AND_CFG_REGISTERS + 1],
            &mut self.rx_buf[0..NR_OF_DATA_AND_CFG_REGISTERS + 1],
        );
        assert!(result.is_ok());
        let x_raw = i16::from_le_bytes(
            self.rx_buf[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2]
                .try_into()
                .unwrap(),
        );
        let y_raw = i16::from_le_bytes(
            self.rx_buf[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2]
                .try_into()
                .unwrap(),
        );
        let z_raw = i16::from_le_bytes(
            self.rx_buf[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2]
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
}

impl<ComInterface: SpiInterface, TmSender: EcssTmSender> ModeProvider
    for MgmHandlerLis3Mdl<ComInterface, TmSender>
{
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl<ComInterface: SpiInterface, TmSender: EcssTmSender> ModeRequestHandler
    for MgmHandlerLis3Mdl<ComInterface, TmSender>
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
            self.shared_mgm_set.lock().unwrap().valid = false;
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

#[cfg(test)]
mod tests {
    use std::sync::{atomic::AtomicU32, mpsc, Arc, Mutex};

    use satrs::{
        mode::{ModeReply, ModeRequest},
        request::{GenericMessage, UniqueApidTargetId},
        tmtc::PacketAsVec,
    };
    use satrs_example::config::components::Apid;
    use satrs_minisim::acs::lis3mdl::MgmLis3RawValues;

    use crate::{pus::hk::HkReply, requests::CompositeRequest};

    use super::*;

    pub struct TestInterface {
        pub call_count: Arc<AtomicU32>,
        pub next_mgm_data: Arc<Mutex<MgmLis3RawValues>>,
    }

    impl TestInterface {
        pub fn new(
            call_count: Arc<AtomicU32>,
            next_mgm_data: Arc<Mutex<MgmLis3RawValues>>,
        ) -> Self {
            Self {
                call_count,
                next_mgm_data,
            }
        }
    }

    impl SpiInterface for TestInterface {
        type Error = ();

        fn transfer(&mut self, _tx: &[u8], rx: &mut [u8]) -> Result<(), Self::Error> {
            let mgm_data = *self.next_mgm_data.lock().unwrap();
            rx[X_LOWBYTE_IDX..X_LOWBYTE_IDX + 2].copy_from_slice(&mgm_data.x.to_le_bytes());
            rx[Y_LOWBYTE_IDX..Y_LOWBYTE_IDX + 2].copy_from_slice(&mgm_data.y.to_be_bytes());
            rx[Z_LOWBYTE_IDX..Z_LOWBYTE_IDX + 2].copy_from_slice(&mgm_data.z.to_be_bytes());
            self.call_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
    }

    pub struct MgmTestbench {
        pub spi_interface_call_count: Arc<AtomicU32>,
        pub next_mgm_data: Arc<Mutex<MgmLis3RawValues>>,
        pub mode_request_tx: mpsc::Sender<GenericMessage<ModeRequest>>,
        pub mode_reply_rx_to_pus: mpsc::Receiver<GenericMessage<ModeReply>>,
        pub mode_reply_rx_to_parent: mpsc::Receiver<GenericMessage<ModeReply>>,
        pub composite_request_tx: mpsc::Sender<GenericMessage<CompositeRequest>>,
        pub hk_reply_rx: mpsc::Receiver<GenericMessage<HkReply>>,
        pub tm_rx: mpsc::Receiver<PacketAsVec>,
        pub handler: MgmHandlerLis3Mdl<TestInterface, mpsc::Sender<PacketAsVec>>,
    }

    impl MgmTestbench {
        pub fn new() -> Self {
            let (request_tx, request_rx) = mpsc::channel();
            let (reply_tx_to_pus, reply_rx_to_pus) = mpsc::channel();
            let (reply_tx_to_parent, reply_rx_to_parent) = mpsc::channel();
            let mode_interface = MpscModeLeafInterface {
                request_rx,
                reply_to_pus_tx: reply_tx_to_pus,
                reply_to_parent_tx: reply_tx_to_parent,
            };
            let (composite_request_tx, composite_request_rx) = mpsc::channel();
            let (hk_reply_tx, hk_reply_rx) = mpsc::channel();
            let (tm_tx, tm_rx) = mpsc::channel::<PacketAsVec>();
            let shared_mgm_set = Arc::default();
            let next_mgm_data = Arc::new(Mutex::default());
            let spi_interface_call_count = Arc::new(AtomicU32::new(0));
            let test_interface =
                TestInterface::new(spi_interface_call_count.clone(), next_mgm_data.clone());
            Self {
                mode_request_tx: request_tx,
                mode_reply_rx_to_pus: reply_rx_to_pus,
                mode_reply_rx_to_parent: reply_rx_to_parent,
                composite_request_tx,
                tm_rx,
                hk_reply_rx,
                spi_interface_call_count,
                next_mgm_data,
                handler: MgmHandlerLis3Mdl::new(
                    UniqueApidTargetId::new(Apid::Acs as u16, 1),
                    "test-mgm",
                    mode_interface,
                    composite_request_rx,
                    hk_reply_tx,
                    tm_tx,
                    test_interface,
                    shared_mgm_set,
                ),
            }
        }
    }

    #[test]
    fn test_basic_handler() {
        let mut testbench = MgmTestbench::new();
        assert_eq!(
            testbench
                .spi_interface_call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            testbench.handler.mode_and_submode().mode(),
            DeviceMode::Off as u32
        );
        assert_eq!(testbench.handler.mode_and_submode().submode(), 0_u16);
        testbench.handler.periodic_operation();
        // Handler is OFF, no changes expected.
        assert_eq!(
            testbench
                .spi_interface_call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
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
                MessageMetadata::new(0, PUS_MODE_SERVICE.id()),
                ModeRequest::SetMode(ModeAndSubmode::new(DeviceMode::Normal as u32, 0)),
            ))
            .expect("failed to send mode request");
        testbench.handler.periodic_operation();
        assert_eq!(
            testbench.handler.mode_and_submode().mode(),
            DeviceMode::Normal as u32
        );
        assert_eq!(testbench.handler.mode_and_submode().submode(), 0);
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
        assert_eq!(
            testbench
                .spi_interface_call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        // TODO: Check shared MGM set. The field values should be 0, but the entry should be valid.
        // TODO: Set non-zero raw values for the next polled MGM set.
    }
}
