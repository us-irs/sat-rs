use std::sync::mpsc::{self};
use std::sync::{Arc, Mutex};

use satrs::mode::{ModeAndSubmode, ModeProvider, ModeReply, ModeRequest, ModeRequestHandler};
use satrs::pus::EcssTmSenderCore;
use satrs::request::GenericMessage;
use satrs::ComponentId;

use crate::pus::hk::HkReply;
use crate::requests::CompositeRequest;

pub trait SpiInterface {
    type Error;
    fn transfer(&mut self, data: &mut [u8]) -> Result<(), Self::Error>;
}

#[derive(Debug, Copy, Clone)]
pub struct MgmData {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub struct MgmHandler<ComInterface: SpiInterface, TmSender: EcssTmSenderCore> {
    id: ComponentId,
    dev_str: &'static str,
    mode_request_receiver: mpsc::Receiver<GenericMessage<ModeRequest>>,
    mode_reply_sender_to_pus: mpsc::Sender<GenericMessage<ModeReply>>,
    mode_reply_sender_to_parent: mpsc::Sender<GenericMessage<ModeReply>>,
    composite_request_receiver: mpsc::Receiver<GenericMessage<CompositeRequest>>,
    hk_reply_sender: mpsc::Sender<GenericMessage<HkReply>>,
    hk_tm_sender: TmSender,
    mode: ModeAndSubmode,
    spi_interface: ComInterface,
    shared_mgm_set: Arc<Mutex<MgmData>>,
}

impl<ComInterface: SpiInterface, TmSender: EcssTmSenderCore> MgmHandler<ComInterface, TmSender> {
    pub fn perform_operation(&mut self) {}
}

impl<ComInterface: SpiInterface, TmSender: EcssTmSenderCore> ModeProvider
    for MgmHandler<ComInterface, TmSender>
{
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode
    }
}

impl<ComInterface: SpiInterface, TmSender: EcssTmSenderCore> ModeRequestHandler
    for MgmHandler<ComInterface, TmSender>
{
    fn start_transition(
        &mut self,
        request_id: satrs::request::RequestId,
        sender_id: ComponentId,
        mode_and_submode: ModeAndSubmode,
    ) -> Result<(), satrs::mode::ModeError> {
        todo!()
    }

    fn announce_mode(
        &self,
        request_id: satrs::request::RequestId,
        sender_id: satrs::ComponentId,
        recursive: bool,
    ) {
        log::info!("{} announcing mode: {:?}", self.dev_str, self.mode);
    }

    fn handle_mode_reached(&mut self) -> Result<(), satrs::queue::GenericTargetedMessagingError> {
        todo!()
    }
}
