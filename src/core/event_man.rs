use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use crate::core::events::{Event, EventRaw, GroupId};

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
enum ListenerType {
    Single(EventRaw),
    Group(GroupId),
}

struct Listener {
    _ltype: ListenerType,
    dest: Sender<Event>,
}

pub struct EventManager {
    listeners: HashMap<ListenerType, Vec<Listener>>,
    event_receiver: Receiver<Event>,
}

impl EventManager {
    pub fn subcribe_single(&mut self, event: Event, dest: Sender<Event>) {
        self.update_listeners(ListenerType::Single(event.raw()), dest);
    }

    pub fn subscribe_group(&mut self, group_id: GroupId, dest: Sender<Event>) {
        self.update_listeners(ListenerType::Group(group_id), dest);
    }

    fn update_listeners(&mut self, key: ListenerType, dest: Sender<Event>) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.listeners.entry(key) {
            e.insert(vec![Listener { _ltype: key, dest }]);
        } else {
            let vec = self.listeners.get_mut(&key).unwrap();
            vec.push(Listener { _ltype: key, dest });
        }
    }

    pub fn periodic_op(&mut self) {
        if let Ok(event) = self.event_receiver.recv() {
            println!("Received event {:?}", event);
            for (ltype, listener_list) in self.listeners.iter() {
                match ltype {
                    ListenerType::Single(raw_event) => {
                        if event.raw() == *raw_event {
                            for listener in listener_list {
                                listener.dest.send(event).expect("Sending event failed");
                            }
                        }
                    }
                    ListenerType::Group(group_id) => {
                        if event.group_id() == *group_id {
                            for listener in listener_list {
                                listener.dest.send(event).expect("Sending event failed");
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

}