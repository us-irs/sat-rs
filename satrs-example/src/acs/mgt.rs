use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};

use types::acs::mgt::{Mode, request::ModeRequest, response::ModeReport};

/// Helper component for communication with a parent component, which is usually an assembly
/// or a subsystem.
pub struct ModeLeafHelper {
    pub request_rx: Receiver<ModeRequest>,
    pub report_tx: SyncSender<ModeReport>,
}

/// Dummy magnetorquer (MGT) device handler. It has no actual actuation logic and reaches any
/// commanded mode immediately, which is sufficient to exercise the mode commanding of its
/// parent subsystem.
pub struct Mgt {
    mode: Mode,
    mode_leaf_helper: ModeLeafHelper,
}

impl Mgt {
    pub fn new(mode_leaf_helper: ModeLeafHelper) -> Self {
        Self {
            mode: Mode::Off,
            mode_leaf_helper,
        }
    }

    #[allow(dead_code)]
    #[inline]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn periodic_operation(&mut self) {
        self.handle_mode_leaf_handling();
    }

    fn handle_mode_leaf_handling(&mut self) {
        loop {
            match self.mode_leaf_helper.request_rx.try_recv() {
                Ok(request) => match request {
                    ModeRequest::SetMode(mode) => {
                        log::info!("MGT: transitioning to mode {:?}", mode);
                        self.mode = mode;
                        self.report_mode();
                    }
                    ModeRequest::ReadMode => self.report_mode(),
                },
                Err(e) => match e {
                    TryRecvError::Empty => break,
                    TryRecvError::Disconnected => log::warn!("packet sender disconnected"),
                },
            }
        }
    }

    fn report_mode(&self) {
        self.mode_leaf_helper
            .report_tx
            .send(ModeReport::Mode(self.mode))
            .expect("failed to send mode report to parent");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    struct MgtTestbench {
        request_tx: SyncSender<ModeRequest>,
        report_rx: Receiver<ModeReport>,
        mgt: Mgt,
    }

    impl MgtTestbench {
        fn new() -> Self {
            let (request_tx, request_rx) = mpsc::sync_channel(5);
            let (report_tx, report_rx) = mpsc::sync_channel(5);
            Self {
                request_tx,
                report_rx,
                mgt: Mgt::new(ModeLeafHelper {
                    request_rx,
                    report_tx,
                }),
            }
        }
    }

    #[test]
    fn test_initial_mode() {
        let testbench = MgtTestbench::new();
        assert_eq!(testbench.mgt.mode(), Mode::Off);
    }

    #[test]
    fn test_set_mode() {
        let mut testbench = MgtTestbench::new();
        testbench
            .request_tx
            .send(ModeRequest::SetMode(Mode::Normal))
            .unwrap();
        testbench.mgt.periodic_operation();
        assert_eq!(testbench.mgt.mode(), Mode::Normal);
        let report = testbench.report_rx.try_recv().expect("no mode report sent");
        assert_eq!(report, ModeReport::Mode(Mode::Normal));
    }

    #[test]
    fn test_read_mode() {
        let mut testbench = MgtTestbench::new();
        testbench.request_tx.send(ModeRequest::ReadMode).unwrap();
        testbench.mgt.periodic_operation();
        let report = testbench.report_rx.try_recv().expect("no mode report sent");
        assert_eq!(report, ModeReport::Mode(Mode::Off));
    }
}
