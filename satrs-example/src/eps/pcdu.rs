use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
};

use derive_new::new;
use satrs::{
    mode::ModeAndSubmode,
    power::SwitchState,
    pus::EcssTmSender,
    request::{GenericMessage, UniqueApidTargetId},
};
use satrs_example::TimestampHelper;

use crate::{acs::mgm::MpscModeLeafInterface, pus::hk::HkReply, requests::CompositeRequest};

pub trait SerialInterface {}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SwitchId {
    Mgm0 = 0,
    Mgt = 1,
}

pub type SwitchMap = HashMap<SwitchId, SwitchState>;

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
    shared_switch_map: Arc<Mutex<SwitchMap>>,
    #[new(value = "ModeAndSubmode::new(satrs_example::DeviceMode::Off as u32, 0)")]
    mode_and_submode: ModeAndSubmode,
    #[new(default)]
    stamp_helper: TimestampHelper,
}
