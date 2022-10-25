#![allow(dead_code, unused_imports)]

use std::convert::AsRef;
use fsrc_core::events::{EventU32, EventU32TypedSev, GenericEvent, HasSeverity, LargestEventRaw, LargestGroupIdRaw, Severity, SeverityInfo, SeverityLow, SeverityMedium};

#[derive(Debug)]
struct GroupIdIntrospection {
    name: &'static str,
    id: LargestGroupIdRaw,
}

#[derive(Debug)]
struct EventIntrospection {
    name: &'static str,
    group_id: GroupIdIntrospection,
    event: &'static EventU32,
    info: &'static str,
}

//#[event(descr="This is some info event")]
const INFO_EVENT_0: EventU32TypedSev<SeverityInfo> = EventU32TypedSev::const_new(0, 0);
const INFO_EVENT_0_ERASED: EventU32 = EventU32::const_from_info(INFO_EVENT_0);

// This is ideally auto-generated
const INFO_EVENT_0_INTROSPECTION: EventIntrospection = EventIntrospection {
    name: "INFO_EVENT_0",
    group_id: GroupIdIntrospection {
        id: 0,
        name: "Group ID 0 without name",
    },
    event: &INFO_EVENT_0_ERASED,
    info: "This is some info event",
};

//#[event(descr="This is some low severity event")]
const SOME_LOW_SEV_EVENT: EventU32TypedSev<SeverityLow> = EventU32TypedSev::const_new(0, 12);

//const EVENT_LIST: [&'static Event; 2] = [&INFO_EVENT_0, &SOME_LOW_SEV_EVENT];

//#[event_group]
const TEST_GROUP_NAME: u16 = 1;
// Auto-generated?
const TEST_GROUP_NAME_NAME: &str = "TEST_GROUP_NAME";

//#[event(desc="Some medium severity event")]
const MEDIUM_SEV_EVENT_IN_OTHER_GROUP: EventU32TypedSev<SeverityMedium> =
    EventU32TypedSev::const_new(TEST_GROUP_NAME, 0);
const MEDIUM_SEV_EVENT_IN_OTHER_GROUP_REDUCED: EventU32 = EventU32::const_from_medium(MEDIUM_SEV_EVENT_IN_OTHER_GROUP);

// Also auto-generated
const MEDIUM_SEV_EVENT_IN_OTHER_GROUP_INTROSPECTION: EventIntrospection =
    EventIntrospection {
        name: "MEDIUM_SEV_EVENT_IN_OTHER_GROUP",
        group_id: GroupIdIntrospection {
            name: TEST_GROUP_NAME_NAME,
            id: TEST_GROUP_NAME,
        },
        event: &MEDIUM_SEV_EVENT_IN_OTHER_GROUP_REDUCED,
        info: "Some medium severity event",
    };

const CONST_SLICE: &'static [u8] = &[0, 1, 2, 3];
const INTROSPECTION_FOR_TEST_GROUP_0: [&EventIntrospection; 2] = [
    &INFO_EVENT_0_INTROSPECTION,
    &INFO_EVENT_0_INTROSPECTION
];

//const INTROSPECTION_FOR_TABLE: &'static [&EventIntrospection] = &INTROSPECTION_FOR_TEST_GROUP_0;

const INTROSPECTION_FOR_TEST_GROUP_NAME: [&EventIntrospection; 1] = [
    &MEDIUM_SEV_EVENT_IN_OTHER_GROUP_INTROSPECTION
];
//const BLAH: &'static [&EventIntrospection] = &INTROSPECTION_FOR_TEST_GROUP_NAME;

const ALL_EVENTS: [&[&EventIntrospection]; 2] = [
    &INTROSPECTION_FOR_TEST_GROUP_0,
    &INTROSPECTION_FOR_TEST_GROUP_NAME
];

#[test]
fn main() {
    //let test = stringify!(INFO_EVENT);
    //println!("{:?}", test);
    //for event in EVENT_LIST {
    //    println!("{:?}", event);
    //}
    for events in ALL_EVENTS.into_iter().flatten() {
        dbg!("{:?}", events);
    }
    //for introspection_info in INTROSPECTION_FOR_TEST_GROUP {
    //    dbg!("{:?}", introspection_info);
    //}
    //let test_struct =
}
