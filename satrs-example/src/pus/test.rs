use crate::pus::AcceptedTc;
use satrs_core::pus::verification::StdVerifReporterWithSender;
use std::sync::mpsc::Receiver;

struct PusService17Handler {
    receiver: Receiver<AcceptedTc>,
    verification_handler: StdVerifReporterWithSender,
}

impl PusService17Handler {
    pub fn periodic_operation(&mut self) {}
}
