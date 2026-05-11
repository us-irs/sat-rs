#![allow(dead_code)]
use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    time::Duration,
};

use models::{
    ComponentId,
    acs::subsystem::{Mode, response},
};
use satrs::{
    mode_tree::{ModeStoreProvider, ModeStoreVec, SequenceModeTables, TargetModeTables},
    spacepackets::CcsdsPacketIdAndPsc,
    subsystem::SubsystemCommandingHelper,
};
use satrs_example::{ModeHelper, TmtcQueues};

#[derive(Debug)]
pub struct TransitionInfo {
    check_mode_reached: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TransitionState {
    #[default]
    Idle,
    AwaitingReplies,
}

#[derive(Debug)]
pub struct ChildModes {
    mgm_assembly_mode: models::acs::mgm_assembly::Mode,
    mgt_mode: models::acs::mgt::Mode,
    controller_mode: models::acs::ctrl::Mode,
}

#[derive(Debug)]
pub struct TransitionCommands {
    mgm_assembly_mode: Option<(models::acs::mgm_assembly::Mode, TransitionInfo)>,
    mgt_mode: Option<(models::acs::mgt::Mode, TransitionInfo)>,
    controller_mode: Option<(models::acs::ctrl::Mode, TransitionInfo)>,
}

const OFF_SEQUENCE: [TransitionCommands; 2] = [
    TransitionCommands {
        mgm_assembly_mode: None,
        mgt_mode: None,
        controller_mode: Some((
            models::acs::ctrl::Mode::Passive,
            TransitionInfo {
                check_mode_reached: true,
            },
        )),
    },
    TransitionCommands {
        mgm_assembly_mode: Some((
            models::acs::mgm_assembly::Mode::Device(models::DeviceMode::Off),
            TransitionInfo {
                check_mode_reached: false,
            },
        )),
        mgt_mode: Some((
            models::acs::mgt::Mode::Off,
            TransitionInfo {
                check_mode_reached: false,
            },
        )),
        controller_mode: None,
    },
];

const SAFE_SEQUENCE: [TransitionCommands; 2] = [
    TransitionCommands {
        mgm_assembly_mode: Some((
            models::acs::mgm_assembly::Mode::Device(models::DeviceMode::Normal),
            TransitionInfo {
                check_mode_reached: false,
            },
        )),
        mgt_mode: Some((
            models::acs::mgt::Mode::Normal,
            TransitionInfo {
                check_mode_reached: false,
            },
        )),
        controller_mode: None,
    },
    TransitionCommands {
        mgm_assembly_mode: None,
        mgt_mode: None,
        controller_mode: Some((
            models::acs::ctrl::Mode::Safe,
            TransitionInfo {
                check_mode_reached: false,
            },
        )),
    },
];

#[derive(Debug)]
pub struct ModeRequestSenders {
    pub mode_request_ctrl: SyncSender<models::acs::ctrl::request::ModeRequest>,
    pub mode_request_assy: SyncSender<models::acs::mgm_assembly::request::ModeRequest>,
    pub mode_request_mgt: SyncSender<models::acs::mgt::request::ModeRequest>,
}

#[derive(Debug)]
pub struct ModeReportReceivers {
    pub mode_response_ctrl: Receiver<models::acs::ctrl::response::ModeReport>,
    pub mode_response_assy: Receiver<models::acs::mgm_assembly::response::ModeResponse>,
    pub mode_response_mgt: Receiver<models::acs::mgt::response::ModeReport>,
}

#[derive(Debug)]
pub struct Subsystem {
    mode_helper: ModeHelper<models::acs::subsystem::Mode, TransitionState>,
    transition_step: usize,
    current_child_modes: Option<ChildModes>,
    mode_request_senders: ModeRequestSenders,
    mode_report_receivers: ModeReportReceivers,
    tmtc_queues: TmtcQueues,
    subsystem_helper: SubsystemCommandingHelper,
}

impl Subsystem {
    pub const ID: ComponentId = ComponentId::AcsSubsystem;

    pub fn new(
        mode_request_senders: ModeRequestSenders,
        mode_report_receivers: ModeReportReceivers,
        tmtc_queues: TmtcQueues,
    ) -> Self {
        let mut mode_store_vec = ModeStoreVec::default();
        mode_store_vec
            .add_component(
                ComponentId::AcsMgmAssembly as satrs::ComponentId,
                models::acs::mgm_assembly::Mode::NoModeKeeping.into(),
            )
            .unwrap();
        mode_store_vec
            .add_component(
                ComponentId::AcsController as satrs::ComponentId,
                models::acs::ctrl::Mode::Passive.into(),
            )
            .unwrap();
        mode_store_vec
            .add_component(
                ComponentId::AcsMgt as satrs::ComponentId,
                models::acs::mgt::Mode::Off.into(),
            )
            .unwrap();

        let target_tables = TargetModeTables::default();

        let sequence_tables = SequenceModeTables::default();

        Self {
            mode_helper: ModeHelper::new(
                models::acs::subsystem::Mode::Off,
                Duration::from_millis(2000),
            ),
            current_child_modes: None,
            mode_request_senders,
            mode_report_receivers,
            tmtc_queues,
            transition_step: 0,
            subsystem_helper: SubsystemCommandingHelper::new(
                mode_store_vec,
                target_tables,
                sequence_tables,
            ),
        }
    }

    pub fn periodic_operation(&mut self) {
        self.handle_telecommands();
    }

    pub fn handle_telecommands(&mut self) {
        loop {
            match self.tmtc_queues.tc_rx.try_recv() {
                Ok(packet) => {
                    let tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&packet.sp_header);
                    match postcard::from_bytes::<models::acs::subsystem::request::Request>(
                        &packet.payload,
                    ) {
                        Ok(request) => match request {
                            models::acs::subsystem::request::Request::Ping => {
                                self.send_telemetry(Some(tc_id), response::Response::Ok)
                            }
                            models::acs::subsystem::request::Request::Mode(mode_request) => {
                                self.handle_mode_request(mode_request);
                            }
                        },
                        Err(e) => {
                            log::warn!("failed to deserialize request: {}", e);
                        }
                    }
                }
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => break,
                    mpsc::TryRecvError::Disconnected => log::warn!("packet sender disconnected"),
                },
            }
        }
    }

    pub fn transition_sequence_for_mode(mode: Mode) -> &'static [TransitionCommands] {
        match mode {
            Mode::Off => &OFF_SEQUENCE,
            Mode::Safe => &SAFE_SEQUENCE,
        }
    }

    pub fn execute_transition_step(&mut self, step: &TransitionCommands) {
        if let Some((target_mode, _info)) = &step.mgm_assembly_mode {
            self.mode_request_senders
                .mode_request_assy
                .send(models::acs::mgm_assembly::request::ModeRequest::SetMode(
                    *target_mode,
                ))
                .expect("failed to send mode request to MGM assembly");
        }
        if let Some((target_mode, _info)) = &step.mgt_mode {
            self.mode_request_senders
                .mode_request_mgt
                .send(models::acs::mgt::request::ModeRequest::SetMode(
                    *target_mode,
                ))
                .expect("failed to send mode request to MGM assembly");
        }
        if let Some((target_mode, _info)) = &step.controller_mode {
            self.mode_request_senders
                .mode_request_ctrl
                .send(models::acs::ctrl::request::ModeRequest::SetMode(
                    *target_mode,
                ))
                .expect("failed to send mode request to MGM assembly");
        }
    }

    pub fn handle_mode_request(
        &mut self,
        mode_request: models::acs::subsystem::request::ModeRequest,
    ) {
        match mode_request {
            models::acs::subsystem::request::ModeRequest::SetMode(target_mode) => {
                self.mode_helper.start(target_mode);
                self.transition_step = 0;
                let first_step = Self::transition_sequence_for_mode(target_mode)
                    .get(self.transition_step)
                    .expect("empty transition table");
                self.execute_transition_step(first_step);
            }
            models::acs::subsystem::request::ModeRequest::ReadMode => {
                self.send_telemetry(
                    None,
                    response::Response::Mode(response::ModeResponse::Mode(
                        self.mode_helper.current,
                    )),
                );
            }
        }
    }

    pub fn send_telemetry(
        &self,
        tc_id: Option<CcsdsPacketIdAndPsc>,
        response: models::acs::subsystem::response::Response,
    ) {
        match crate::ccsds::pack_ccsds_tm_packet_for_now(Self::ID, tc_id, &response) {
            Ok(packet) => {
                if let Err(e) = self.tmtc_queues.tm_tx.send(packet) {
                    log::warn!("failed to send TM packet: {}", e);
                }
            }
            Err(e) => {
                log::warn!("failed to pack TM packet: {}", e);
            }
        }
    }
}
