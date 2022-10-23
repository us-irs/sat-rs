#![allow(dead_code, unused_imports)]
use fsrc_core::events::{Event, EventProvider, LargestEventRaw, LargestGroupIdRaw, Severity};

struct GroupIdIntrospection {
    name: &'static str,
    id: LargestGroupIdRaw,
}

struct EventIntrospection {
    name: &'static str,
    group_id: GroupIdIntrospection,
    event: &'static Event,
    info: &'static str,
}

//#[event(descr="This is some info event")]
const INFO_EVENT_0: Event = Event::const_new(Severity::INFO, 0, 0);

// This is ideally auto-generated
const INFO_EVENT_0_INTROSPECTION: EventIntrospection = EventIntrospection {
    name: "INFO_EVENT_0",
    group_id: GroupIdIntrospection {
        id: 0,
        name: "Group ID 0 without name",
    },
    event: &INFO_EVENT_0,
    info: "This is some info event",
};

//#[event(descr="This is some low severity event")]
const SOME_LOW_SEV_EVENT: Event = Event::const_new(Severity::LOW, 0, 12);

const EVENT_LIST: [&'static Event; 2] = [&INFO_EVENT_0, &SOME_LOW_SEV_EVENT];

//#[event_group]
const TEST_GROUP_NAME: u16 = 1;
// Auto-generated?
const TEST_GROUP_NAME_NAME: &'static str = "TEST_GROUP_NAME";

//#[event(desc="Some medium severity event")]
const MEDIUM_SEV_EVENT_IN_OTHER_GROUP: Event =
    Event::const_new(Severity::MEDIUM, TEST_GROUP_NAME, 0);

// Also auto-generated
const MEDIUM_SEV_EVENT_IN_OTHER_GROUP_INTROSPECTION: EventIntrospection = EventIntrospection {
    name: "MEDIUM_SEV_EVENT_IN_OTHER_GROUP",
    group_id: GroupIdIntrospection {
        name: TEST_GROUP_NAME_NAME,
        id: TEST_GROUP_NAME,
    },
    event: &MEDIUM_SEV_EVENT_IN_OTHER_GROUP,
    info: "Some medium severity event",
};

#[test]
fn main() {
    let test = stringify!(INFO_EVENT);
    println!("{:?}", test);
    for event in EVENT_LIST {
        println!("{:?}", event);
    }
    //let test_struct =
}
