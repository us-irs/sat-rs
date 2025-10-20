use arbitrary_int::u11;
use satrs::event_man_legacy::{
    EventManagerWithMpsc, EventMessage, EventMessageU32, EventRoutingError, EventSendProvider,
    EventU32SenderMpsc,
};
use satrs::events_legacy::{EventU32, EventU32TypedSev, Severity, SeverityInfo};
use satrs::params::U32Pair;
use satrs::params::{Params, ParamsHeapless, WritableToBeBytes};
use satrs::pus::event_man::{DefaultPusEventReportingMap, EventReporter, PusEventTmCreatorWithMap};
use satrs::request::UniqueApidTargetId;
use satrs::tmtc::PacketAsVec;
use spacepackets::ecss::PusError;
use spacepackets::ecss::tm::PusTmReader;
use std::sync::mpsc::{self, SendError, TryRecvError};
use std::thread;

const INFO_EVENT: EventU32TypedSev<SeverityInfo> = EventU32TypedSev::<SeverityInfo>::new(1, 0);
const LOW_SEV_EVENT: EventU32 = EventU32::new(Severity::Low, 1, 5);
const EMPTY_STAMP: [u8; 7] = [0; 7];
const TEST_APID: u11 = u11::new(0x02);
const TEST_ID: UniqueApidTargetId = UniqueApidTargetId::new(TEST_APID, 0x05);

#[derive(Debug, Clone)]
pub enum CustomTmSenderError {
    SendError(SendError<Vec<u8>>),
    PusError(PusError),
}

#[test]
fn test_threaded_usage() {
    let (event_tx, event_rx) = mpsc::sync_channel(100);
    let mut event_man = EventManagerWithMpsc::new(event_rx);

    let (pus_event_man_tx, pus_event_man_rx) = mpsc::channel();
    let pus_event_man_send_provider = EventU32SenderMpsc::new(1, pus_event_man_tx);
    event_man.subscribe_all(pus_event_man_send_provider.target_id());
    event_man.add_sender(pus_event_man_send_provider);
    let (event_packet_tx, event_packet_rx) = mpsc::channel::<PacketAsVec>();
    let reporter = EventReporter::new(TEST_ID.raw(), u11::new(0x02), 0, 128);
    let pus_event_man =
        PusEventTmCreatorWithMap::new(reporter, DefaultPusEventReportingMap::default());
    let error_handler = |event_msg: &EventMessageU32, error: EventRoutingError| {
        panic!("received routing error for event {event_msg:?}: {error:?}");
    };
    // PUS + Generic event manager thread
    let jh0 = thread::spawn(move || {
        let mut event_cnt = 0;
        let mut params_array: [u8; 128] = [0; 128];
        loop {
            event_man.try_event_handling(error_handler);
            match pus_event_man_rx.try_recv() {
                Ok(event_msg) => {
                    let gen_event = |aux_data| {
                        pus_event_man.generate_pus_event_tm_generic(
                            &event_packet_tx,
                            &EMPTY_STAMP,
                            event_msg.event(),
                            aux_data,
                        )
                    };
                    let res = if let Some(aux_data) = event_msg.params() {
                        match aux_data {
                            Params::Heapless(heapless) => match heapless {
                                ParamsHeapless::Raw(raw) => {
                                    raw.write_to_be_bytes(&mut params_array)
                                        .expect("Writing raw parameter failed");
                                    gen_event(Some(&params_array[0..raw.written_len()]))
                                }
                                ParamsHeapless::EcssEnum(e) => {
                                    e.write_to_be_bytes(&mut params_array)
                                        .expect("Writing ECSS enum failed");
                                    gen_event(Some(&params_array[0..e.written_len()]))
                                }
                            },
                            Params::Vec(vec) => gen_event(Some(vec.as_slice())),
                            Params::String(str) => gen_event(Some(str.as_bytes())),
                            Params::Store(_) => gen_event(None),
                            _ => panic!("unsupported parameter type"),
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
        event_tx
            .send(EventMessage::new(TEST_ID.id(), INFO_EVENT.into()))
            .expect("Sending info event failed");
        loop {
            match event_packet_rx.try_recv() {
                // Event TM received successfully
                Ok(event_tm) => {
                    let tm = PusTmReader::new(event_tm.packet.as_slice(), 7)
                        .expect("Deserializing TM failed");
                    assert_eq!(tm.service(), 5);
                    assert_eq!(tm.subservice(), 1);
                    let src_data = tm.source_data();
                    assert!(!src_data.is_empty());
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
        event_tx
            .send(EventMessage::new_with_params(
                TEST_ID.id(),
                LOW_SEV_EVENT,
                &Params::Heapless((2_u32, 3_u32).into()),
            ))
            .expect("Sending low severity event failed");
        loop {
            match event_packet_rx.try_recv() {
                // Event TM received successfully
                Ok(event_tm) => {
                    let tm = PusTmReader::new(event_tm.packet.as_slice(), 7)
                        .expect("Deserializing TM failed");
                    assert_eq!(tm.service(), 5);
                    assert_eq!(tm.subservice(), 2);
                    let src_data = tm.source_data();
                    assert!(!src_data.is_empty());
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
