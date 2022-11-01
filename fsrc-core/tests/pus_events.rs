use fsrc_core::event_man::{EventManager, MpscEventReceiver, MpscEventU32SendProvider};
use fsrc_core::events::{EventU32, EventU32TypedSev, Severity, SeverityInfo};
use fsrc_core::pus::event_man::{DefaultPusMgmtBackendProvider, EventReporter, PusEventTmManager};
use fsrc_core::pus::{EcssTmError, EcssTmSender};
use fsrc_core::util::Params;
use spacepackets::tm::PusTm;
use std::sync::mpsc::{channel, SendError, TryRecvError};
use std::thread;

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
fn test_threaded_usage() {
    let (event_sender, event_man_receiver) = channel();
    let event_receiver = MpscEventReceiver::new(event_man_receiver);
    let mut event_man = EventManager::new(Box::new(event_receiver));

    let (pus_event_man_tx, pus_event_man_rx) = channel();
    let pus_event_man_send_provider = MpscEventU32SendProvider::new(1, pus_event_man_tx);
    event_man.subscribe_all(pus_event_man_send_provider);
    let (event_tx, event_rx) = channel();
    let reporter = EventReporter::new(0x02, 128).expect("Creating event reporter failed");
    let backend = DefaultPusMgmtBackendProvider::<EventU32>::default();
    let mut pus_event_man = PusEventTmManager::new(reporter, Box::new(backend));
    // PUS + Generic event manager thread
    let jh0 = thread::spawn(move || {
        let mut sender = EventTmSender { sender: event_tx };
        loop {
            let res = event_man.try_event_handling();
            assert!(res.is_ok());
            match pus_event_man_rx.try_recv() {
                Ok((event, aux_data)) => {
                    // TODO: Convert auxiliary data into raw byte format
                    if let Some(aux_data) = aux_data {
                        match aux_data {
                            Params::Heapless(_) => {}
                            Params::Vec(_) => {}
                            Params::String(_) => {}
                        }
                    }
                    let res = pus_event_man.generate_pus_event_tm_generic(
                        &mut sender,
                        &EMPTY_STAMP,
                        event,
                        None,
                    );
                    assert!(res.is_ok());
                    assert!(res.unwrap());
                    break;
                }
                Err(e) => {
                    if let TryRecvError::Disconnected = e {
                        panic!("Event receiver disconnected!")
                    }
                }
            }
        }
    });

    // Event sender and TM checker thread
    let jh1 = thread::spawn(move || {
        event_sender
            .send((INFO_EVENT.into(), None))
            .expect("Sending info event failed");
        loop {
            match event_rx.try_recv() {
                // Event TM received successfully
                Ok(_) => break,
                Err(e) => {
                    if let TryRecvError::Disconnected = e {
                        panic!("Event sender disconnected!")
                    }
                }
            }
        }
        event_sender
            .send((
                LOW_SEV_EVENT.into(),
                Some(Params::Heapless((2_u32, 3_u32).into())),
            ))
            .expect("Sending low severity event failed");
        loop {
            match event_rx.try_recv() {
                // Event TM received successfully
                Ok(_) => break,
                Err(e) => {
                    if let TryRecvError::Disconnected = e {
                        panic!("Event sender disconnected!")
                    }
                }
            }
        }
    });
    jh0.join().expect("Joining manager thread failed");
    jh1.join().expect("Joining creator thread failed");
}
