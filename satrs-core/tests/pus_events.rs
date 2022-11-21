use satrs_core::event_man::{
    EventManagerWithMpscQueue, MpscEventU32Receiver, MpscEventU32SendProvider, SendEventProvider,
};
use satrs_core::events::{EventU32, EventU32TypedSev, Severity, SeverityInfo};
use satrs_core::params::U32Pair;
use satrs_core::params::{Params, ParamsHeapless, WritableToBeBytes};
use satrs_core::pus::event_man::{
    DefaultPusMgmtBackendProvider, EventReporter, PusEventDispatcher,
};
use satrs_core::pus::{EcssTmError, EcssTmSender};
use spacepackets::ecss::PusPacket;
use spacepackets::tm::PusTm;
use std::sync::mpsc::{channel, SendError, TryRecvError};
use std::thread;

const INFO_EVENT: EventU32TypedSev<SeverityInfo> =
    EventU32TypedSev::<SeverityInfo>::const_new(1, 0);
const LOW_SEV_EVENT: EventU32 = EventU32::const_new(Severity::LOW, 1, 5);
const EMPTY_STAMP: [u8; 7] = [0; 7];

#[derive(Clone)]
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
    let event_receiver = MpscEventU32Receiver::new(event_man_receiver);
    let mut event_man = EventManagerWithMpscQueue::new(Box::new(event_receiver));

    let (pus_event_man_tx, pus_event_man_rx) = channel();
    let pus_event_man_send_provider = MpscEventU32SendProvider::new(1, pus_event_man_tx);
    event_man.subscribe_all(pus_event_man_send_provider.id());
    event_man.add_sender(pus_event_man_send_provider);
    let (event_tx, event_rx) = channel();
    let reporter = EventReporter::new(0x02, 128).expect("Creating event reporter failed");
    let backend = DefaultPusMgmtBackendProvider::<EventU32>::default();
    let mut pus_event_man = PusEventDispatcher::new(reporter, Box::new(backend));
    // PUS + Generic event manager thread
    let jh0 = thread::spawn(move || {
        let mut sender = EventTmSender { sender: event_tx };
        let mut event_cnt = 0;
        let mut params_array: [u8; 128] = [0; 128];
        loop {
            let res = event_man.try_event_handling();
            assert!(res.is_ok());
            match pus_event_man_rx.try_recv() {
                Ok((event, aux_data)) => {
                    let mut gen_event = |aux_data| {
                        pus_event_man.generate_pus_event_tm_generic(
                            &mut sender,
                            &EMPTY_STAMP,
                            event,
                            aux_data,
                        )
                    };
                    let res = if let Some(aux_data) = aux_data {
                        match aux_data {
                            Params::Heapless(heapless) => match heapless {
                                ParamsHeapless::Raw(raw) => {
                                    raw.write_to_be_bytes(&mut params_array)
                                        .expect("Writing raw parameter failed");
                                    gen_event(Some(&params_array[0..raw.raw_len()]))
                                }
                                ParamsHeapless::EcssEnum(e) => {
                                    e.write_to_be_bytes(&mut params_array)
                                        .expect("Writing ECSS enum failed");
                                    gen_event(Some(&params_array[0..e.raw_len()]))
                                }
                                ParamsHeapless::Store(_) => gen_event(None),
                            },
                            Params::Vec(vec) => gen_event(Some(vec.as_slice())),
                            Params::String(str) => gen_event(Some(str.as_bytes())),
                        }
                    } else {
                        gen_event(None)
                    };
                    event_cnt += 1;
                    assert!(res.is_ok());
                    assert!(res.unwrap());
                    if event_cnt == 2 {
                        break;
                    }
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
                Ok(event_tm) => {
                    let tm =
                        PusTm::from_bytes(event_tm.as_slice(), 7).expect("Deserializing TM failed");
                    assert_eq!(tm.0.service(), 5);
                    assert_eq!(tm.0.subservice(), 1);
                    let src_data = tm.0.source_data();
                    assert!(src_data.is_some());
                    let src_data = src_data.unwrap();
                    assert_eq!(src_data.len(), 4);
                    let event =
                        EventU32::from(u32::from_be_bytes(src_data[0..4].try_into().unwrap()));
                    assert_eq!(event, INFO_EVENT);
                    break;
                }
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
                Ok(event_tm) => {
                    let tm =
                        PusTm::from_bytes(event_tm.as_slice(), 7).expect("Deserializing TM failed");
                    assert_eq!(tm.0.service(), 5);
                    assert_eq!(tm.0.subservice(), 2);
                    let src_data = tm.0.source_data();
                    assert!(src_data.is_some());
                    let src_data = src_data.unwrap();
                    assert_eq!(src_data.len(), 12);
                    let event =
                        EventU32::from(u32::from_be_bytes(src_data[0..4].try_into().unwrap()));
                    assert_eq!(event, LOW_SEV_EVENT);
                    let u32_pair: U32Pair =
                        src_data[4..].try_into().expect("Creating U32Pair failed");
                    assert_eq!(u32_pair.0, 2);
                    assert_eq!(u32_pair.1, 3);
                    break;
                }
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
