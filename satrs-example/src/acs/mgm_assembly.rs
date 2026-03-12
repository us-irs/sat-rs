// TODO: Program assembly.
// TODO: Remove dead_code lint as soon as assembly is done.
#![allow(dead_code)]

use std::{sync::mpsc, time::Duration};

use models::DeviceMode;
use satrs_example::{ModeHelper, TmtcQueues};

pub enum ModeRequest {
    SetMode(AssemblyMode),
    ReadMode,
}

pub enum ModeReport {
    /// Mode of the assembly.
    Mode(AssemblyMode),
    /// Failure setting the children mode.
    SetModeRetryLimitReached([DeviceMode; 2]),
    /// An assembly can not keep its mode.
    CanNotKeepMode([DeviceMode; 2]),
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
    CommandingChildren {
        step: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssemblyMode {
    /// The assembly mode ressembles the modes of the devices it controls. It also tries to keep
    /// the children in the correct mode by re-commanding them into the correct mode.
    Device(DeviceMode),
    /// Mode keeping disabled.
    NoModeKeeping,
}

/// MGM assembly component.
pub struct Assembly {
    mode_helper: ModeHelper<AssemblyMode, TransitionState>,
    /// This boolean is used for the distinction between transitions commanded by the parent
    /// or by ground, and transitions which were commanded autonomously as part of children
    /// mode keeping.
    mode_keeping_transition: bool,
    tmtc_queues: TmtcQueues,
    mgm_modes: [DeviceMode; 2],
    parent_queues: ParentQueueHelper,
    pub(crate) children_queues: ChildrenQueueHelper,
}

impl Assembly {
    const RETRIES: usize = 3;

    pub fn new(
        parent_queues: ParentQueueHelper,
        children_queues: ChildrenQueueHelper,
        tmtc_queues: TmtcQueues,
    ) -> Self {
        Self {
            mode_helper: ModeHelper::new(AssemblyMode::NoModeKeeping, Duration::from_millis(200)),
            mode_keeping_transition: false,
            tmtc_queues,
            mgm_modes: [DeviceMode::Off; 2],
            parent_queues,
            children_queues,
        }
    }

    pub fn periodic_operation(&mut self) {
        self.handle_parent_mode_queue();
        self.handle_children_mode_queues();

        if self.mode_helper.transition_active() {
            self.handle_mode_transition();
        }
    }

    pub fn handle_mode_transition(&mut self) {
        if self.mode_helper.transition_state == TransitionState::Idle {
            self.mode_helper.transition_state = TransitionState::CommandingChildren { step: 0 };
        }
        if let TransitionState::CommandingChildren { step } = self.mode_helper.transition_state
            && self.mode_helper.timed_out()
        {
            if step >= Self::RETRIES {
                let report = if self.mode_keeping_transition {
                    ModeReport::CanNotKeepMode(self.mgm_modes)
                } else {
                    ModeReport::SetModeRetryLimitReached(self.mgm_modes)
                };
                self.parent_queues.report_tx.send(report).unwrap();
                self.mode_helper.finish(false);
            }
            if let AssemblyMode::Device(device_mode) = self.mode_helper.target.unwrap() {
                self.command_children(device_mode);
            } else {
                self.mode_helper.finish(true);
            }
            self.mode_helper.transition_state =
                TransitionState::CommandingChildren { step: step + 1 }
        }
    }

    pub fn command_children(&self, mode: DeviceMode) {
        for tx in &self.children_queues.request_tx_queues {
            tx.send(super::mgm::ModeRequest::SetMode(mode)).unwrap();
        }
    }

    pub fn handle_parent_mode_queue(&mut self) {
        loop {
            match self.parent_queues.request_rx.try_recv() {
                Ok(request) => match request {
                    ModeRequest::SetMode(assembly_mode) => match assembly_mode {
                        AssemblyMode::Device(device_mode) => {
                            self.mode_keeping_transition = false;
                            self.mode_helper.start(AssemblyMode::Device(device_mode));
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
        for (idx, rx) in self.children_queues.report_rx_queues.iter_mut().enumerate() {
            loop {
                match rx.try_recv() {
                    Ok(report) => match report {
                        super::mgm::ModeReport::Mode(device_mode) => {
                            self.mgm_modes[idx] = device_mode;

                            // Transition is active, check for completion.
                            // If at least one child reached the correct mode, we are done.
                            if self.mode_helper.transition_active()
                                && let AssemblyMode::Device(device_mode) =
                                    self.mode_helper.target.unwrap()
                                && self.mgm_modes.contains(&device_mode)
                            {
                                self.mode_helper.finish(true);
                            }

                            // Mode keeping active: Check children modes.
                            if let AssemblyMode::Device(device_mode) = self.mode_helper.current
                                && self.mgm_modes.iter().all(|m| *m != device_mode)
                            {
                                self.mode_keeping_transition = true;
                                // Children lost mode. Try to command them back to the correct
                                // mode.
                                self.mode_helper.start(self.mode_helper.current);
                            }
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
    }
}
