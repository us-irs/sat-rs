//! [Event][crate::core::events::Event] management and forwarding
use crate::core::events::{Event, EventRaw, GroupId};
use std::collections::HashMap;

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
enum ListenerType {
    Single(EventRaw),
    Group(GroupId),
}

pub trait EventListener {
    type Error;

    fn id(&self) -> u32;
    fn send_to(&mut self, event: Event) -> Result<(), Self::Error>;
}

struct Listener<E> {
    ltype: ListenerType,
    dest: Box<dyn EventListener<Error = E>>,
}

pub trait ReceivesAllEvent {
    fn receive(&mut self) -> Option<Event>;
}

pub struct EventManager<E> {
    listeners: HashMap<ListenerType, Vec<Listener<E>>>,
    event_receiver: Box<dyn ReceivesAllEvent>,
}

pub enum HandlerResult {
    Empty,
    Handled(u32, Event),
}

impl<E> EventManager<E> {
    pub fn new(event_receiver: Box<dyn ReceivesAllEvent>) -> Self {
        EventManager {
            listeners: HashMap::new(),
            event_receiver,
        }
    }
    pub fn subscribe_single(
        &mut self,
        event: Event,
        dest: impl EventListener<Error = E> + 'static,
    ) {
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
                ltype: key,
                dest: Box::new(dest),
            }]);
        } else {
            let vec = self.listeners.get_mut(&key).unwrap();
            // To prevent double insertions
            for entry in vec.iter() {
                if entry.ltype == key && entry.dest.id() == dest.id() {
                    return;
                }
            }
            vec.push(Listener {
                ltype: key,
                dest: Box::new(dest),
            });
        }
    }

    pub fn try_event_handling(&mut self) -> Result<HandlerResult, E> {
        let mut err_status = None;
        let mut num_recipients = 0;
        let mut send_handler = |event, llist: &mut Vec<Listener<E>>| {
            for listener in llist.iter_mut() {
                if let Err(e) = listener.dest.send_to(event) {
                    err_status = Some(Err(e));
                } else {
                    num_recipients += 1;
                }
            }
        };
        if let Some(event) = self.event_receiver.receive() {
            let single_key = ListenerType::Single(event.raw());
            if self.listeners.contains_key(&single_key) {
                send_handler(event, self.listeners.get_mut(&single_key).unwrap());
            }
            let group_key = ListenerType::Group(event.group_id());
            if self.listeners.contains_key(&group_key) {
                send_handler(event, self.listeners.get_mut(&group_key).unwrap());
            }
            if let Some(err) = err_status {
                return err;
            }
            return Ok(HandlerResult::Handled(num_recipients, event));
        }
        Ok(HandlerResult::Empty)
    }
}

#[cfg(test)]
mod tests {
    use super::{EventListener, HandlerResult, ReceivesAllEvent};
    use crate::core::event_man::EventManager;
    use crate::core::events::{Event, Severity};
    use std::sync::mpsc::{channel, Receiver, SendError, Sender};
    use std::thread;
    use std::time::Duration;

    struct EventReceiver {
        mpsc_receiver: Receiver<Event>,
    }
    impl ReceivesAllEvent for EventReceiver {
        fn receive(&mut self) -> Option<Event> {
            self.mpsc_receiver.try_recv().ok()
        }
    }

    #[derive(Clone)]
    struct MpscEventSenderQueue {
        id: u32,
        mpsc_sender: Sender<Event>,
    }

    impl EventListener for MpscEventSenderQueue {
        type Error = SendError<Event>;

        fn id(&self) -> u32 {
            self.id
        }
        fn send_to(&mut self, event: Event) -> Result<(), Self::Error> {
            self.mpsc_sender.send(event)
        }
    }

    fn check_next_event(expected: Event, receiver: &Receiver<Event>) {
        for _ in 0..5 {
            if let Ok(event) = receiver.try_recv() {
                assert_eq!(event, expected);
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn check_handled_event(res: HandlerResult, expected: Event, expected_num_sent: u32) {
        assert!(matches!(res, HandlerResult::Handled { .. }));
        if let HandlerResult::Handled(num_recipients, event) = res {
            assert_eq!(event, expected);
            assert_eq!(num_recipients, expected_num_sent);
        }
    }

    #[test]
    fn test_basic() {
        let (event_sender, manager_queue) = channel();
        let event_man_receiver = EventReceiver {
            mpsc_receiver: manager_queue,
        };
        let mut event_man: EventManager<SendError<Event>> =
            EventManager::new(Box::new(event_man_receiver));
        let event_grp_0 = Event::new(Severity::INFO, 0, 0).unwrap();
        let event_grp_1_0 = Event::new(Severity::HIGH, 1, 0).unwrap();
        let (single_event_sender, single_event_receiver) = channel();
        let single_event_listener = MpscEventSenderQueue {
            id: 0,
            mpsc_sender: single_event_sender,
        };
        event_man.subscribe_single(event_grp_0, single_event_listener);
        let (group_event_sender_0, group_event_receiver_0) = channel();
        let group_event_listener = MpscEventSenderQueue {
            id: 1,
            mpsc_sender: group_event_sender_0,
        };
        event_man.subscribe_group(event_grp_1_0.group_id(), group_event_listener);

        // Test event with one listener
        event_sender
            .send(event_grp_0)
            .expect("Sending single error failed");
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_grp_0, 1);
        check_next_event(event_grp_0, &single_event_receiver);

        // Test event which is sent to all group listeners
        event_sender
            .send(event_grp_1_0)
            .expect("Sending group error failed");
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_grp_1_0, 1);
        check_next_event(event_grp_1_0, &group_event_receiver_0);
    }

    /// Test listening for multiple groups
    #[test]
    fn test_multi_group() {
        let (event_sender, manager_queue) = channel();
        let event_man_receiver = EventReceiver {
            mpsc_receiver: manager_queue,
        };
        let mut event_man: EventManager<SendError<Event>> =
            EventManager::new(Box::new(event_man_receiver));
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        let hres = res.unwrap();
        assert!(matches!(hres, HandlerResult::Empty));

        let event_grp_0 = Event::new(Severity::INFO, 0, 0).unwrap();
        let event_grp_1_0 = Event::new(Severity::HIGH, 1, 0).unwrap();
        let (event_grp_0_sender, event_grp_0_receiver) = channel();
        let event_grp_0_and_1_listener = MpscEventSenderQueue {
            id: 0,
            mpsc_sender: event_grp_0_sender,
        };
        event_man.subscribe_group(event_grp_0.group_id(), event_grp_0_and_1_listener.clone());
        event_man.subscribe_group(event_grp_1_0.group_id(), event_grp_0_and_1_listener);

        event_sender
            .send(event_grp_0)
            .expect("Sending Event Group 0 failed");
        event_sender
            .send(event_grp_1_0)
            .expect("Sendign Event Group 1 failed");
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_grp_0, 1);
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_grp_1_0, 1);

        check_next_event(event_grp_0, &event_grp_0_receiver);
        check_next_event(event_grp_1_0, &event_grp_0_receiver);
    }

    /// Test listening to the same event from multiple listeners. Also test listening
    /// to both group and single events from one listener
    #[test]
    fn test_listening_to_same_event_and_multi_type() {
        let (event_sender, manager_queue) = channel();
        let event_man_receiver = EventReceiver {
            mpsc_receiver: manager_queue,
        };
        let mut event_man: EventManager<SendError<Event>> =
            EventManager::new(Box::new(event_man_receiver));
        let event_0 = Event::new(Severity::INFO, 0, 5).unwrap();
        let event_1 = Event::new(Severity::HIGH, 1, 0).unwrap();
        let (event_0_tx_0, event_0_rx_0) = channel();
        let (event_0_tx_1, event_0_rx_1) = channel();
        let event_listener_0 = MpscEventSenderQueue {
            id: 0,
            mpsc_sender: event_0_tx_0,
        };
        let event_listener_1 = MpscEventSenderQueue {
            id: 1,
            mpsc_sender: event_0_tx_1,
        };
        event_man.subscribe_single(event_0, event_listener_0.clone());
        event_man.subscribe_single(event_0, event_listener_1);
        event_sender
            .send(event_0)
            .expect("Triggering Event 0 failed");
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_0, 2);
        check_next_event(event_0, &event_0_rx_0);
        check_next_event(event_0, &event_0_rx_1);
        event_man.subscribe_group(event_1.group_id(), event_listener_0.clone());
        event_sender
            .send(event_0)
            .expect("Triggering Event 0 failed");
        event_sender
            .send(event_1)
            .expect("Triggering Event 1 failed");

        // 3 Events messages will be sent now
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_0, 2);
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_1, 1);
        // Both the single event and the group event should arrive now
        check_next_event(event_0, &event_0_rx_0);
        check_next_event(event_1, &event_0_rx_0);

        // Double insertion should be detected, result should remain the same
        event_man.subscribe_group(event_1.group_id(), event_listener_0);
        event_sender
            .send(event_1)
            .expect("Triggering Event 1 failed");
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_1, 1);
    }
}
