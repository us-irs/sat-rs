//! Event management and forwarding
use crate::events::{EventU16TypedSev, EventU32, GenericEvent, HasSeverity};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use hashbrown::HashMap;

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
enum ListenerType {
    Single(u32),
    Group(u16),
}

pub trait EventListener<Provider: GenericEvent> {
    type Error;

    fn id(&self) -> u32;
    fn send_to(&mut self, event: Provider) -> Result<(), Self::Error>;
}

struct Listener<E, Provider: GenericEvent> {
    ltype: ListenerType,
    dest: Box<dyn EventListener<Provider, Error = E>>,
}

pub trait ReceivesAllEvent<Provider: GenericEvent> {
    fn receive(&mut self) -> Option<Provider>;
}

pub struct EventManager<E, Provider: GenericEvent> {
    listeners: HashMap<ListenerType, Vec<Listener<E, Provider>>>,
    event_receiver: Box<dyn ReceivesAllEvent<Provider>>,
}

pub enum HandlerResult<Provider: GenericEvent> {
    Empty,
    Handled(u32, Provider),
}

impl<E> EventManager<E, EventU32> {
    pub fn new(event_receiver: Box<dyn ReceivesAllEvent<EventU32>>) -> Self {
        EventManager {
            listeners: HashMap::new(),
            event_receiver,
        }
    }
}

impl<E> EventManager<E, EventU32> {
    pub fn subscribe_single(
        &mut self,
        event: EventU32,
        dest: impl EventListener<EventU32, Error = E> + 'static,
    ) {
        self.update_listeners(ListenerType::Single(event.raw_as_largest_type()), dest);
    }

    pub fn subscribe_group(
        &mut self,
        group_id: <EventU32 as GenericEvent>::GroupId,
        dest: impl EventListener<EventU32, Error = E> + 'static,
    ) {
        self.update_listeners(ListenerType::Group(group_id), dest);
    }
}

impl<E, SEVERITY: HasSeverity + Copy> EventManager<E, EventU16TypedSev<SEVERITY>> {
    pub fn subscribe_single(
        &mut self,
        event: EventU16TypedSev<SEVERITY>,
        dest: impl EventListener<EventU16TypedSev<SEVERITY>, Error = E> + 'static,
    ) {
        self.update_listeners(ListenerType::Single(event.raw_as_largest_type()), dest);
    }

    pub fn subscribe_group(
        &mut self,
        group_id: <EventU16TypedSev<SEVERITY> as GenericEvent>::GroupId,
        dest: impl EventListener<EventU16TypedSev<SEVERITY>, Error = E> + 'static,
    ) {
        self.update_listeners(ListenerType::Group(group_id.into()), dest);
    }
}

impl<E, Provider: GenericEvent + Copy> EventManager<E, Provider> {
    fn update_listeners(
        &mut self,
        key: ListenerType,
        dest: impl EventListener<Provider, Error = E> + 'static,
    ) {
        if !self.listeners.contains_key(&key) {
            self.listeners.insert(
                key,
                vec![Listener {
                    ltype: key,
                    dest: Box::new(dest),
                }],
            );
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

    pub fn try_event_handling(&mut self) -> Result<HandlerResult<Provider>, E> {
        let mut err_status = None;
        let mut num_recipients = 0;
        let mut send_handler = |event: Provider, llist: &mut Vec<Listener<E, Provider>>| {
            for listener in llist.iter_mut() {
                if let Err(e) = listener.dest.send_to(event) {
                    err_status = Some(Err(e));
                } else {
                    num_recipients += 1;
                }
            }
        };
        if let Some(event) = self.event_receiver.receive() {
            let single_key = ListenerType::Single(event.raw_as_largest_type());
            if self.listeners.contains_key(&single_key) {
                send_handler(event, self.listeners.get_mut(&single_key).unwrap());
            }
            let group_key = ListenerType::Group(event.group_id_as_largest_type());
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
    use crate::event_man::EventManager;
    use crate::events::{EventU32, GenericEvent, Severity};
    use alloc::boxed::Box;
    use std::sync::mpsc::{channel, Receiver, SendError, Sender};
    use std::thread;
    use std::time::Duration;

    struct EventReceiver {
        mpsc_receiver: Receiver<EventU32>,
    }
    impl ReceivesAllEvent<EventU32> for EventReceiver {
        fn receive(&mut self) -> Option<EventU32> {
            self.mpsc_receiver.try_recv().ok()
        }
    }

    #[derive(Clone)]
    struct MpscEventSenderQueue {
        id: u32,
        mpsc_sender: Sender<EventU32>,
    }

    impl EventListener<EventU32> for MpscEventSenderQueue {
        type Error = SendError<EventU32>;

        fn id(&self) -> u32 {
            self.id
        }
        fn send_to(&mut self, event: EventU32) -> Result<(), Self::Error> {
            self.mpsc_sender.send(event)
        }
    }

    fn check_next_event(expected: EventU32, receiver: &Receiver<EventU32>) {
        for _ in 0..5 {
            if let Ok(event) = receiver.try_recv() {
                assert_eq!(event, expected);
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn check_handled_event(
        res: HandlerResult<EventU32>,
        expected: EventU32,
        expected_num_sent: u32,
    ) {
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
        let mut event_man: EventManager<SendError<EventU32>, EventU32> =
            EventManager::new(Box::new(event_man_receiver));
        let event_grp_0 = EventU32::new(Severity::INFO, 0, 0).unwrap();
        let event_grp_1_0 = EventU32::new(Severity::HIGH, 1, 0).unwrap();
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
        let mut event_man: EventManager<SendError<EventU32>, EventU32> =
            EventManager::new(Box::new(event_man_receiver));
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        let hres = res.unwrap();
        assert!(matches!(hres, HandlerResult::Empty));

        let event_grp_0 = EventU32::new(Severity::INFO, 0, 0).unwrap();
        let event_grp_1_0 = EventU32::new(Severity::HIGH, 1, 0).unwrap();
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
        let mut event_man: EventManager<SendError<EventU32>, EventU32> =
            EventManager::new(Box::new(event_man_receiver));
        let event_0 = EventU32::new(Severity::INFO, 0, 5).unwrap();
        let event_1 = EventU32::new(Severity::HIGH, 1, 0).unwrap();
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
