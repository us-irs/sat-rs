use derive_new::new;
use satrs::hk::{HkRequest, HkRequestVariant};
use satrs::queue::{GenericSendError, GenericTargetedMessagingError};
use satrs::spacepackets::ecss::hk;
use satrs::spacepackets::ecss::tm::{PusTmCreator, PusTmSecondaryHeader};
use satrs::spacepackets::SpHeader;
use satrs_example::{DeviceMode, TimestampHelper};
use satrs_minisim::acs::lis3mdl::{FIELD_LSB_PER_GAUSS_4_SENS, GAUSS_TO_MICROTESLA_FACTOR};
use satrs_minisim::acs::MgmRequestLis3Mdl;
use satrs_minisim::{SimReply, SimRequest};
use std::sync::mpsc::{self};
use std::sync::{Arc, Mutex};

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
    fn transfer(&mut self, _tx: &[u8], _rx: &mut [u8]) -> Result<(), Self::Error> {
        let mgm_sensor_request = MgmRequestLis3Mdl::RequestSensorData;
        self.sim_request_tx
            .send(SimRequest::new_with_epoch_time(mgm_sensor_request))
            .expect("failed to send request");
        self.sim_reply_rx.recv().expect("reply timeout");
        // TODO: Write the sensor data to the raw buffer.
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
    pub reply_tx_to_pus: mpsc::Sender<GenericMessage<ModeReply>>,
    pub reply_tx_to_parent: mpsc::Sender<GenericMessage<ModeReply>>,
}

/// Example MGM device handler strongly based on the LIS3MDL MEMS device.
#[derive(new)]
#[allow(clippy::too_many_arguments)]
pub struct MgmHandlerLis3Mdl<ComInterface: SpiInterface, TmSender: EcssTmSender> {
    id: UniqueApidTargetId,
    dev_str: &'static str,
    mode_interface: MpscModeLeafInterface,
    composite_request_receiver: mpsc::Receiver<GenericMessage<CompositeRequest>>,
    hk_reply_sender: mpsc::Sender<GenericMessage<HkReply>>,
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
            // Communicate with the device.
            let result = self.com_interface.transfer(
                &self.tx_buf[0..NR_OF_DATA_AND_CFG_REGISTERS + 1],
                &mut self.rx_buf[0..NR_OF_DATA_AND_CFG_REGISTERS + 1],
            );
            assert!(result.is_ok());
            // Actual data begins on the second byte, similarly to how a lot of SPI devices behave.
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
            // Simple scaling to retrieve the float value, assuming a sensor resolution of
            let mut mgm_guard = self.shared_mgm_set.lock().unwrap();
            mgm_guard.x =
                x_raw as f32 * GAUSS_TO_MICROTESLA_FACTOR as f32 * FIELD_LSB_PER_GAUSS_4_SENS;
            mgm_guard.y =
                y_raw as f32 * GAUSS_TO_MICROTESLA_FACTOR as f32 * FIELD_LSB_PER_GAUSS_4_SENS;
            mgm_guard.z =
                z_raw as f32 * GAUSS_TO_MICROTESLA_FACTOR as f32 * FIELD_LSB_PER_GAUSS_4_SENS;
            drop(mgm_guard);
        }
    }

    pub fn handle_composite_requests(&mut self) {
        loop {
            match self.composite_request_receiver.try_recv() {
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
                self.hk_reply_sender
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
            .reply_tx_to_pus
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
    // TODO: Add some basic tests for the modes of the device.
}
