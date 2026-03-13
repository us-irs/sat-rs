#![allow(dead_code)]
use std::{sync::mpsc, time::Duration};

use models::{
    ComponentId, DeviceMode,
    mgm_assembly::{
        AssemblyMode,
        response::{self, ModeCommandFailure},
    },
};
use satrs::spacepackets::CcsdsPacketIdAndPsc;
use satrs_example::{ModeHelper, TmtcQueues};

use crate::ccsds::pack_ccsds_tm_packet_for_now;

#[derive(Debug, Copy, Clone)]
pub enum ModeRequest {
    SetMode(AssemblyMode),
    ReadMode,
}

#[derive(Debug, Copy, Clone)]
pub enum ModeReport {
    /// Mode of the assembly.
    Mode(AssemblyMode),
    /// Failure setting the children mode.
    SetModeTimeout([Option<DeviceMode>; 2]),
    /// An assembly tried modekeeping but can not keep its mode.
    CanNotKeepMode([Option<DeviceMode>; 2]),
}

pub struct ParentQueueHelper {
    pub request_rx: mpsc::Receiver<ModeRequest>,
    pub report_tx: mpsc::SyncSender<ModeReport>,
}

/// Helper component for communication with a parent component, which is usually as assembly.
pub struct ChildrenQueueHelper {
    pub request_tx_queues: [mpsc::SyncSender<super::mgm::ModeRequest>; 2],
    pub report_rx_queues: [mpsc::Receiver<super::mgm::ModeReport>; 2],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TransitionState {
    #[default]
    Idle,
    AwaitingReplies,
}

#[derive(Debug, Default, Copy, Clone)]
pub struct MgmInfo {
    reply_received: bool,
    mode: Option<DeviceMode>,
}

/// MGM assembly component.
pub struct Assembly {
    mode_helper: ModeHelper<AssemblyMode, TransitionState>,
    /// This boolean is used for the distinction between transitions commanded by the parent
    /// or by ground, and transitions which were commanded autonomously as part of children
    /// mode keeping.
    mode_keeping_transition: bool,
    tmtc_queues: TmtcQueues,
    mgm_modes: [MgmInfo; 2],
    parent_queues: ParentQueueHelper,
    pub(crate) children_queues: ChildrenQueueHelper,
}

impl Assembly {
    const ID: ComponentId = ComponentId::AcsMgmAssembly;

    pub fn new(
        parent_queues: ParentQueueHelper,
        children_queues: ChildrenQueueHelper,
        tmtc_queues: TmtcQueues,
        mode_timeout: Duration,
    ) -> Self {
        Self {
            mode_helper: ModeHelper::new(AssemblyMode::NoModeKeeping, mode_timeout),
            mode_keeping_transition: false,
            tmtc_queues,
            mgm_modes: [MgmInfo::default(); 2],
            parent_queues,
            children_queues,
        }
    }

    pub fn periodic_operation(&mut self) {
        self.handle_telecommands();
        self.handle_parent_mode_queue();
        self.handle_children_mode_queues();

        if self.mode_helper.transition_active() {
            self.handle_mode_transition();
        }
    }

    pub fn handle_telecommands(&mut self) {
        loop {
            match self.tmtc_queues.tc_rx.try_recv() {
                Ok(packet) => {
                    let tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&packet.sp_header);
                    match postcard::from_bytes::<models::mgm_assembly::request::Request>(
                        &packet.payload,
                    ) {
                        Ok(request) => match request {
                            models::mgm_assembly::request::Request::Ping => {
                                self.send_telemetry(Some(tc_id), response::Response::Ok)
                            }
                            models::mgm_assembly::request::Request::Mode(assembly_mode) => {
                                self.start_transition(false, assembly_mode, Some(tc_id))
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

    pub fn send_telemetry(
        &self,
        tc_id: Option<CcsdsPacketIdAndPsc>,
        response: models::mgm_assembly::response::Response,
    ) {
        match pack_ccsds_tm_packet_for_now(Self::ID, tc_id, &response) {
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

    pub fn handle_parent_mode_queue(&mut self) {
        loop {
            match self.parent_queues.request_rx.try_recv() {
                Ok(request) => match request {
                    ModeRequest::SetMode(assembly_mode) => match assembly_mode {
                        AssemblyMode::Device(_device_mode) => {
                            self.start_transition(false, assembly_mode, None);
                        }
                        AssemblyMode::NoModeKeeping => {
                            self.mode_helper.current = AssemblyMode::NoModeKeeping
                        }
                    },
                    ModeRequest::ReadMode => self
                        .parent_queues
                        .report_tx
                        .send(ModeReport::Mode(self.mode_helper.current))
                        .unwrap(),
                },
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => break,
                    mpsc::TryRecvError::Disconnected => {
                        log::warn!("packet sender disconnected")
                    }
                },
            }
        }
    }

    pub fn handle_children_mode_queues(&mut self) {
        let mut mode_report_received = false;
        for (idx, rx) in self.children_queues.report_rx_queues.iter_mut().enumerate() {
            loop {
                match rx.try_recv() {
                    Ok(report) => match report {
                        super::mgm::ModeReport::Mode(device_mode) => {
                            self.mgm_modes[idx].mode = Some(device_mode);
                            self.mgm_modes[idx].reply_received = true;
                            mode_report_received = true;
                        }
                        super::mgm::ModeReport::SetModeTimeout => {
                            // Ignore, handle this with our own timeout.
                            log::warn!("MGM {} mode timeout", idx);
                        }
                    },
                    Err(e) => match e {
                        mpsc::TryRecvError::Empty => break,
                        mpsc::TryRecvError::Disconnected => {
                            log::warn!("packet sender disconnected")
                        }
                    },
                }
            }
        }
        if !mode_report_received {
            return;
        }

        // Transition is active, check for completion.
        // If at least one child reached the correct mode, we are done.
        if self.mode_helper.transition_active()
            && let AssemblyMode::Device(device_mode) = self.mode_helper.target.unwrap()
            && self.mgm_modes.iter().all(|i| i.reply_received)
            && self.mgm_modes.iter().any(|i| i.mode == Some(device_mode))
        {
            self.handle_mode_reached();
        }

        // Mode keeping active: Check children modes.
        if let AssemblyMode::Device(device_mode) = self.mode_helper.current
            && self
                .mgm_modes
                .iter()
                .all(|info| info.mode != Some(device_mode))
        {
            // Children lost mode. Try to command them back to the correct
            // mode.
            self.start_transition(true, self.mode_helper.current, None);
        }
    }

    pub fn handle_mode_transition(&mut self) {
        if self.mode_helper.target.is_none() {
            self.handle_mode_reached();
            return;
        }
        let target = self.mode_helper.target.unwrap();
        let device_mode = match target {
            AssemblyMode::Device(device_mode) => device_mode,
            AssemblyMode::NoModeKeeping => {
                self.handle_mode_reached();
                return;
            }
        };
        if self.mode_helper.transition_state == TransitionState::Idle {
            self.command_children(device_mode);
            self.mode_helper.transition_state = TransitionState::AwaitingReplies;
        }
        if self.mode_helper.transition_state == TransitionState::AwaitingReplies
            && self.mode_helper.timed_out()
        {
            self.handle_mode_transition_failure();
        }
    }

    pub fn handle_mode_reached(&mut self) {
        self.announce_mode();
        if self.mode_helper.tc_id.is_some() {
            self.send_telemetry(self.mode_helper.tc_id, response::Response::Ok);
        }
        self.parent_queues
            .report_tx
            .send(ModeReport::Mode(self.mode_helper.current))
            .unwrap();
        self.mode_helper.finish(true);
    }

    pub fn handle_mode_transition_failure(&mut self) {
        let report = if self.mode_keeping_transition {
            ModeReport::CanNotKeepMode(self.mgm_modes.map(|info| info.mode))
        } else {
            ModeReport::SetModeTimeout(self.mgm_modes.map(|info| info.mode))
        };
        if self.mode_helper.tc_id.is_some() {
            self.send_telemetry(
                self.mode_helper.tc_id,
                response::Response::ModeFailure(ModeCommandFailure::Timeout),
            );
        }
        self.parent_queues.report_tx.send(report).unwrap();
        self.mode_helper.finish(false);
    }

    pub fn command_children(&self, mode: DeviceMode) {
        for tx in &self.children_queues.request_tx_queues {
            tx.send(super::mgm::ModeRequest::SetMode(mode)).unwrap();
        }
    }

    pub fn start_transition(
        &mut self,
        mode_keeping: bool,
        target: AssemblyMode,
        tc_id: Option<CcsdsPacketIdAndPsc>,
    ) {
        self.mode_keeping_transition = mode_keeping;
        self.mode_helper.tc_id = tc_id;
        self.mgm_modes
            .iter_mut()
            .for_each(|m| m.reply_received = false);
        self.mode_helper.start(target);
    }

    fn announce_mode(&self) {
        // TODO: Event?
        log::info!(
            "{:?} announcing mode: {:?}",
            Self::ID,
            self.mode_helper.current
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::TryRecvError;

    use models::ccsds::{CcsdsTcPacketOwned, CcsdsTmPacketOwned};

    use super::*;

    pub struct Testbench {
        subsystem_req_tx: mpsc::SyncSender<ModeRequest>,
        subsystem_report_rx: mpsc::Receiver<ModeReport>,
        mgm_request_rx: [mpsc::Receiver<crate::mgm::ModeRequest>; 2],
        mgm_report_tx: [mpsc::SyncSender<crate::mgm::ModeReport>; 2],
        tc_tx: mpsc::SyncSender<CcsdsTcPacketOwned>,
        tm_rx: mpsc::Receiver<CcsdsTmPacketOwned>,
        assembly: Assembly,
    }

    impl Testbench {
        pub fn new() -> Self {
            let (subsystem_req_tx, subsystem_req_rx) = mpsc::sync_channel(5);
            let (subsystem_report_tx, subsystem_report_rx) = mpsc::sync_channel(5);

            let (mgm_0_mode_request_tx, mgm_0_mode_request_rx) = mpsc::sync_channel(5);
            let (mgm_1_mode_request_tx, mgm_1_mode_request_rx) = mpsc::sync_channel(5);
            let (mgm_0_mode_report_tx, mgm_0_mode_report_rx) = mpsc::sync_channel(5);
            let (mgm_1_mode_report_tx, mgm_1_mode_report_rx) = mpsc::sync_channel(5);

            let (tc_tx, tc_rx) = mpsc::sync_channel(5);
            let (tm_tx, tm_rx) = mpsc::sync_channel(5);

            Self {
                subsystem_req_tx,
                subsystem_report_rx,
                mgm_request_rx: [mgm_0_mode_request_rx, mgm_1_mode_request_rx],
                mgm_report_tx: [mgm_0_mode_report_tx, mgm_1_mode_report_tx],
                tc_tx,
                tm_rx,
                assembly: Assembly::new(
                    ParentQueueHelper {
                        request_rx: subsystem_req_rx,
                        report_tx: subsystem_report_tx,
                    },
                    ChildrenQueueHelper {
                        request_tx_queues: [mgm_0_mode_request_tx, mgm_1_mode_request_tx],
                        report_rx_queues: [mgm_0_mode_report_rx, mgm_1_mode_report_rx],
                    },
                    TmtcQueues { tc_rx, tm_tx },
                    Duration::from_millis(20),
                ),
            }
        }

        pub fn assert_all_queues_empty(&self) {
            assert!(
                matches!(self.tm_rx.try_recv().unwrap_err(), TryRecvError::Empty),
                "TM queue not empty"
            );
            assert!(
                matches!(
                    self.subsystem_report_rx.try_recv().unwrap_err(),
                    TryRecvError::Empty
                ),
                "subsystem report queue not empty"
            );
            for rx in self.mgm_request_rx.iter() {
                assert!(
                    matches!(rx.try_recv().unwrap_err(), TryRecvError::Empty),
                    "mgm request queue not empty"
                )
            }
        }
    }

    #[test]
    fn basic_test() {
        let mut tb = Testbench::new();
        tb.assert_all_queues_empty();
        tb.assembly.periodic_operation();
    }
}
