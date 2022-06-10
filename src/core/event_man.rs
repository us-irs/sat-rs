use crate::core::events::{Event, EventRaw, GroupId};
use std::collections::HashMap;

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
enum ListenerType {
    Single(EventRaw),
    Group(GroupId),
}

pub trait EventListener {
    type Error;
    fn send_to(&mut self, event: Event) -> Result<(), Self::Error>;
}

struct Listener<E> {
    _ltype: ListenerType,
    dest: Box<dyn EventListener<Error = E>>,
}

pub trait ReceivesAllEvent {
    fn receive(&mut self) -> Option<Event>;
}

pub struct EventManager<E> {
    listeners: HashMap<ListenerType, Vec<Listener<E>>>,
    event_receiver: Box<dyn ReceivesAllEvent>,
}

impl<E> EventManager<E> {
    pub fn new(event_receiver: Box<dyn ReceivesAllEvent>) -> Self {
        EventManager {
            listeners: HashMap::new(),
            event_receiver,
        }
    }
    pub fn subcribe_single(&mut self, event: Event, dest: impl EventListener<Error = E> + 'static) {
        self.update_listeners(ListenerType::Single(event.raw()), dest);
    }

    pub fn subscribe_group(
        &mut self,
        group_id: GroupId,
        dest: impl EventListener<Error = E> + 'static,
    ) {
        self.update_listeners(ListenerType::Group(group_id), dest);
    }

    fn update_listeners(
        &mut self,
        key: ListenerType,
        dest: impl EventListener<Error = E> + 'static,
    ) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.listeners.entry(key) {
            e.insert(vec![Listener {
                _ltype: key,
                dest: Box::new(dest),
            }]);
        } else {
            let vec = self.listeners.get_mut(&key).unwrap();
            vec.push(Listener {
                _ltype: key,
                dest: Box::new(dest),
            });
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
    use super::{EventListener, ReceivesAllEvent};
    use crate::core::event_man::EventManager;
    use crate::core::events::{Event, Severity};
    use std::fmt::Error;
    use std::sync::mpsc::{channel, Receiver, SendError, Sender};

    struct EventReceiver {
        mpsc_receiver: Receiver<Event>,
    }
    impl ReceivesAllEvent for EventReceiver {
        fn receive(&mut self) -> Option<Event> {
            self.mpsc_receiver.recv().ok()
        }
    }

    struct MpscEventSenderQueue {
        mpsc_sender: Sender<Event>,
    }

    impl EventListener for MpscEventSenderQueue {
        type Error = SendError<Event>;

        fn send_to(&mut self, event: Event) -> Result<(), Self::Error> {
            self.mpsc_sender.send(event)
        }
    }

    #[test]
    fn basic_test() {
        let (event_sender, manager_queue) = channel();
        let event_man_receiver = EventReceiver {
            mpsc_receiver: manager_queue,
        };
        let mut event_man: EventManager<SendError<Event>> =
            EventManager::new(Box::new(event_man_receiver));
        let sender_0 = event_sender.clone();
        let event_grp_0 = Event::new(Severity::INFO, 0, 0).unwrap();
        let event_grp_1_0 = Event::new(Severity::HIGH, 1, 0).unwrap();
        let event_grp_1_1 = Event::new(Severity::MEDIUM, 1, 32).unwrap();
        let event_grp_1_2 = Event::new(Severity::LOW, 1, 43).unwrap();
        let event_grp_2 = Event::new(Severity::HIGH, 32, 0).unwrap();
        let (single_event_sender, single_event_receiver) = channel();
        let single_event_listener = MpscEventSenderQueue {
            mpsc_sender: single_event_sender,
        };
        let (group_event_sender, group_event_receiver) = channel();
        let group_event_listener = MpscEventSenderQueue {
            mpsc_sender: group_event_sender,
        };
        event_man.subscribe_group(event_grp_1_0.group_id(), group_event_listener);
        event_sender.send(event_grp_0).expect("Send error occured");
        // let event = single_event_receiver.recv().expect("Error receiving event");
        // println!("{:?}", event);
    }
}
