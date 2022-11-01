use fsrc_core::events::{EventU32, EventU32TypedSev, Severity, SeverityInfo};
use fsrc_core::pus::event_man::{DefaultPusMgmtBackendProvider, EventReporter, PusEventTmManager};
use fsrc_core::pus::{EcssTmError, EcssTmSender};
use spacepackets::tm::PusTm;
use std::sync::mpsc::{channel, SendError, TryRecvError};

const INFO_EVENT: EventU32TypedSev<SeverityInfo> =
    EventU32TypedSev::<SeverityInfo>::const_new(1, 0);
const LOW_SEV_EVENT: EventU32 = EventU32::const_new(Severity::LOW, 1, 5);
const EMPTY_STAMP: [u8; 7] = [0; 7];

struct EventTmSender {
    sender: std::sync::mpsc::Sender<Vec<u8>>,
}

impl EcssTmSender for EventTmSender {
    type Error = SendError<Vec<u8>>;
    fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmError<Self::Error>> {
        let mut vec = Vec::new();
        tm.append_to_vec(&mut vec)?;
        self.sender.send(vec).map_err(EcssTmError::SendError)?;
        Ok(())
    }
}

#[test]
fn test_basic() {
    let reporter = EventReporter::new(0x02, 128).expect("Creating event repoter failed");
    let backend = DefaultPusMgmtBackendProvider::<EventU32>::default();
    let mut event_man = PusEventTmManager::new(reporter, Box::new(backend));
    let (event_tx, event_rx) = channel();
    let mut sender = EventTmSender { sender: event_tx };
    let mut event_sent = event_man
        .generate_pus_event_tm(&mut sender, &EMPTY_STAMP, INFO_EVENT, None)
        .expect("Sending info event failed");

    assert!(event_sent);
    // Will not check packet here, correctness of packet was tested somewhere else
    event_rx.recv().expect("Receiving event TM failed");
    let res = event_man.disable_tm_for_event_with_sev(&INFO_EVENT);
    assert!(res.is_ok());
    assert!(res.unwrap());
    event_sent = event_man
        .generate_pus_event_tm(&mut sender, &EMPTY_STAMP, INFO_EVENT, None)
        .expect("Sending info event failed");
    assert!(!event_sent);
    let res = event_rx.try_recv();
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), TryRecvError::Empty));
}
