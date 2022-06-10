use std::collections::HashMap;
use crate::core::events::{Event, EventRaw, GroupId};

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
enum ListenerType {
    Single(EventRaw),
    Group(GroupId),
}

pub trait EventRecipient {
    type Error;
    fn send_to(&mut self, event: Event) -> Result<(), Self::Error>;
}

struct Listener<E> {
    _ltype: ListenerType,
    dest: Box<dyn EventRecipient<Error=E>>,
}

pub trait ReceivesEvent {
    fn receive(&mut self) -> Option<Event>;
}


pub struct EventManager<E> {
    listeners: HashMap<ListenerType, Vec<Listener<E>>>,
    event_receiver: dyn ReceivesEvent,
}

impl<E> EventManager<E> {
    pub fn subcribe_single(&mut self, event: Event, dest: impl EventRecipient<Error=E> + 'static) {
        self.update_listeners(ListenerType::Single(event.raw()), dest);
    }

    pub fn subscribe_group(&mut self, group_id: GroupId, dest: impl EventRecipient<Error=E> + 'static) {
        self.update_listeners(ListenerType::Group(group_id), dest);
    }

    fn update_listeners (&mut self, key: ListenerType, dest: impl EventRecipient<Error=E> + 'static) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.listeners.entry(key) {
            e.insert(vec![Listener { _ltype: key, dest: Box::new(dest) }]);
        } else {
            let vec = self.listeners.get_mut(&key).unwrap();
            vec.push(Listener { _ltype: key, dest: Box::new(dest) });
        }
    }

    pub fn handle_one_event(&mut self) -> Result<(), E> {
        let mut status = Ok(());
        if let Some(event) = self.event_receiver.receive() {
            for (ltype, listener_list) in self.listeners.iter_mut() {
                match ltype {
                    ListenerType::Single(raw_event) => {
                        if event.raw() == *raw_event {
                            for listener in listener_list.iter_mut() {
                                if let Err(e) = listener.dest.send_to(event) {
                                    status = Err(e);
                                }
                            }
                        }
                    }
                    ListenerType::Group(group_id) => {
                        if event.group_id() == *group_id {
                            for listener in listener_list.iter_mut() {
                                if let Err(e) = listener.dest.send_to(event) {
                                    status = Err(e);
                                }
                            }
                        }
                    }
                }
            }
        }
        status
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn basic_test() {

    }
}