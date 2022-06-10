use launchpad::core::events::{Event, EventRaw, GroupId, Severity};
use launchpad::core::executable::{Executable, ExecutionType, OpResult};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
enum ListenerType {
    SINGLE(EventRaw),
    GROUP(GroupId),
}

struct Listener {
    _ltype: ListenerType,
    dest: Sender<Event>,
}

pub struct EventManager {
    listeners: HashMap<ListenerType, Vec<Listener>>,
    event_receiver: Receiver<Event>,
}

impl Executable for EventManager {
    type Error = ();

    fn exec_type(&self) -> ExecutionType {
        ExecutionType::Infinite
    }

    fn task_name(&self) -> &'static str {
        "Event Manager"
    }

    fn periodic_op(&mut self, _op_code: i32) -> Result<OpResult, Self::Error> {
        if let Ok(event) = self.event_receiver.recv() {
            println!("Received event {:?}", event);
        }
        Ok(OpResult::Ok)
    }
}

impl EventManager {
    pub fn subcribe_single(&mut self, event: Event, dest: Sender<Event>) {
        self.update_listeners(ListenerType::SINGLE(event.raw()), dest);
    }

    pub fn subscribe_group(&mut self, group_id: GroupId, dest: Sender<Event>) {
        self.update_listeners(ListenerType::GROUP(group_id), dest);
    }

    fn update_listeners(&mut self, key: ListenerType, dest: Sender<Event>) {
        if !self.listeners.contains_key(&key) {
            self.listeners
                .insert(key.clone(), vec![Listener { _ltype: key, dest }]);
        } else {
            let vec = self.listeners.get_mut(&key).unwrap();
            vec.push(Listener { _ltype: key, dest });
        }
    }
}
fn main() {
    basic_event_test();
}

fn basic_event_test() {
    let test_event_0 = Event::new(Severity::INFO, 0, 0);
    let (event_sender, event_receiver) = channel();
    let t0_sender = event_sender.clone();
    let t0 = thread::spawn(move || {
        t0_sender
            .send(test_event_0.unwrap())
            .expect("Sending event from t0 failed");
    });

    let test_event_1 = Event::new(Severity::MEDIUM, 1, 8);
    let t1 = thread::spawn(move || {
        event_sender
            .send(test_event_1.unwrap())
            .expect("Sending event from t1 failed");
    });

    let mut event_cntr = 0;
    while event_cntr < 2 {
        if let Ok(event) = event_receiver.recv() {
            println!("Received event {:?}", event);
            event_cntr += 1;
        }
    }
    t0.join().unwrap();
    t1.join().unwrap();
}
