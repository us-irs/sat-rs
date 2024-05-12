use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
};

use derive_new::new;
use satrs::{
    hk::{HkRequest, HkRequestVariant},
    mode::{ModeAndSubmode, ModeError, ModeProvider, ModeReply, ModeRequestHandler},
    power::SwitchState,
    pus::EcssTmSender,
    queue::{GenericSendError, GenericTargetedMessagingError},
    request::{GenericMessage, MessageMetadata, UniqueApidTargetId},
};
use satrs_example::{config::components::PUS_MODE_SERVICE, DeviceMode, TimestampHelper};

use crate::{acs::mgm::MpscModeLeafInterface, pus::hk::HkReply, requests::CompositeRequest};

pub trait SerialInterface {
    type Error;
    /// Send some data via the serial interface.
    fn send(&self, data: &[u8]) -> Result<(), Self::Error>;
    /// Receive all replies received on the serial interface so far. This function takes a closure
    /// and call its for each received packet, passing the received packet into it.
    fn recv_replies<ReplyHandler: FnMut(&[u8])>(&self, f: ReplyHandler) -> Result<(), Self::Error>;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum OpCode {
    RegularOp = 0,
    PollAndRecvReplies = 1,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SwitchId {
    Mgm0 = 0,
    Mgt = 1,
}

pub type SwitchMap = HashMap<SwitchId, SwitchState>;

#[derive(Clone, PartialEq, Eq)]
pub struct SwitchSet {
    pub valid: bool,
    pub switch_map: SwitchMap,
}

/// Example PCDU device handler.
#[derive(new)]
#[allow(clippy::too_many_arguments)]
pub struct PcduHandler<ComInterface: SerialInterface, TmSender: EcssTmSender> {
    id: UniqueApidTargetId,
    dev_str: &'static str,
    mode_interface: MpscModeLeafInterface,
    composite_request_rx: mpsc::Receiver<GenericMessage<CompositeRequest>>,
    hk_reply_tx: mpsc::Sender<GenericMessage<HkReply>>,
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
            }
            OpCode::PollAndRecvReplies => {}
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
