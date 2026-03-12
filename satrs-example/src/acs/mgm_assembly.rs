// TODO: Program assembly.
// TODO: Remove dead_code lint as soon as assembly is done.
#![allow(dead_code)]

use std::sync::mpsc;

use models::DeviceMode;

pub enum ModeRequest {
    SetMode(DeviceMode),
    ReadMode,
}

pub enum ModeReport {
    Mode(DeviceMode),
}

/// Helper component for communication with a parent component, which is usually as assembly.
pub struct QueueHelper {
    pub request_tx_queues: [mpsc::SyncSender<super::mgm_assembly::ModeRequest>; 2],
    pub report_rx_queues: [mpsc::Receiver<super::mgm_assembly::ModeReport>; 2],
}

pub struct Assembly {
    pub(crate) helper: QueueHelper,
}

impl Assembly {
    pub fn periodic_operation(&mut self) {
        self.handle_mode_queue();
    }

    pub fn handle_mode_queue(&mut self) {
        for rx in &mut self.helper.report_rx_queues {
            loop {
                match rx.try_recv() {
                    // TODO: Do something with the report.
                    Ok(report) => match report {
                        ModeReport::Mode(_device_mode) => (),
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
