// TODO: Program assembly.
// TODO: Remove dead_code lint as soon as assembly is done.
#![allow(dead_code)]

use std::{sync::mpsc, time::Duration};

use models::DeviceMode;
use satrs_example::ModeHelper;

pub enum ModeRequest {
    SetMode(DeviceMode),
    ReadMode,
}

pub enum ModeReport {
    Mode(DeviceMode),
    /// Setting a mode timed out.
    SetModeTimeout,
    /// An assembly child lost the mode.
    ChildLostMode,
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

/// MGM assembly component.
pub struct Assembly {
    mode_helper: ModeHelper<DeviceMode>,
    parent_queues: ParentQueueHelper,
    pub(crate) children_queues: ChildrenQueueHelper,
}

impl Assembly {
    pub fn new(parent_queues: ParentQueueHelper, children_queues: ChildrenQueueHelper) -> Self {
        Self {
            mode_helper: ModeHelper::new(DeviceMode::Unknown, Duration::from_millis(200)),
            parent_queues,
            children_queues,
        }
    }

    pub fn periodic_operation(&mut self) {
        self.handle_children_mode_queues();
    }

    pub fn handle_children_mode_queues(&mut self) {
        for rx in &mut self.children_queues.report_rx_queues {
            loop {
                match rx.try_recv() {
                    // TODO: Do something with the report.
                    Ok(report) => match report {
                        super::mgm::ModeReport::Mode(_device_mode) => todo!(),
                        super::mgm::ModeReport::SetModeTimeout => todo!(),
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
