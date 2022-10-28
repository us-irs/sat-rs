//! Event management and forwarding
//!
//! This module provides components to perform event routing. The most important component for this
//! task is the [EventManager]. It has a map of event listeners and uses a dynamic [EventReceiver]
//! instance to receive all events and then route them to event subscribers where appropriate.
//!
//! One common use case for satellite systems is to offer a light-weight publish-subscribe mechanism
//! and IPC mechanism for software and hardware events which are also packaged as telemetry.
//! This can be done with the [EventManager] like this:
//!
//!  1. Provide a concrete [SendEventProvider] implementation and a concrete [EventReceiver]
//!     implementation. These abstraction allow to use different message queue backends.
//!     A straightforward implementation where dynamic memory allocation is not a big concern could
//!     use [std::sync::mpsc::channel] to do this. It is recommended that these implementations
//!     derive [Clone].
//!  2. Each event creator gets a sender component which allows it to send events to the manager.
//!  3. The event manager receives all receiver ends so all events are routed to the
//!     manager.
//!  4. Each event receiver and/or subscriber gets a receiver component. The sender component is
//!     used with the [SendEventProvider] trait and the subscription API provided by the
//!     [EventManager] to subscribe for individual events or whole group of events.
use crate::events::{GenericEvent, LargestEventRaw, LargestGroupIdRaw};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use hashbrown::HashMap;

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
enum ListenerType {
    Single(LargestEventRaw),
    Group(LargestGroupIdRaw),
    All,
}

pub trait SendEventProvider<Provider: GenericEvent> {
    type Error;

    fn id(&self) -> u32;
    fn send_no_data(&mut self, event: Provider) -> Result<(), Self::Error> {
        self.send(event, None)
    }
    fn send(&mut self, event: Provider, aux_data: Option<&[u8]>) -> Result<(), Self::Error>;
}

struct Listener<E, Event: GenericEvent> {
    ltype: ListenerType,
    send_provider: Box<dyn SendEventProvider<Event, Error = E>>,
}

/// Generic abstraction for an event receiver.
pub trait EventReceiver<Event: GenericEvent> {
    /// This function has to be provided by any event receiver. A receive call may or may not return
    /// an event.
    ///
    /// To allow returning arbitrary additional auxiliary data, a mutable slice is passed to the
    /// [Self::receive] call as well. Receivers can write data to this slice, but care must be taken
    /// to avoid panics due to size missmatches or out of bound writes.
    fn receive(&mut self, aux_data: &mut [u8]) -> Option<Event>;
}

/// Generic event manager implementation.
pub struct EventManager<E, Event: GenericEvent> {
    aux_data_buf: Vec<u8>,
    listeners: HashMap<ListenerType, Vec<Listener<E, Event>>>,
    event_receiver: Box<dyn EventReceiver<Event>>,
}

pub enum HandlerResult<Provider: GenericEvent> {
    Empty,
    Handled(u32, Provider),
}

impl<E, Event: GenericEvent + Copy> EventManager<E, Event> {
    pub fn new(event_receiver: Box<dyn EventReceiver<Event>>, buf_len_aux_data: usize) -> Self {
        EventManager {
            aux_data_buf: vec![0; buf_len_aux_data],
            listeners: HashMap::new(),
            event_receiver,
        }
    }
    pub fn subscribe_single(
        &mut self,
        event: Event,
        dest: impl SendEventProvider<Event, Error = E> + 'static,
    ) {
        self.update_listeners(ListenerType::Single(event.raw_as_largest_type()), dest);
    }

    pub fn subscribe_group(
        &mut self,
        group_id: LargestGroupIdRaw,
        dest: impl SendEventProvider<Event, Error = E> + 'static,
    ) {
        self.update_listeners(ListenerType::Group(group_id), dest);
    }

    pub fn subscribe_all(&mut self, dest: impl SendEventProvider<Event, Error = E> + 'static) {
        self.update_listeners(ListenerType::All, dest);
    }

    /// Helper function which removes single subscriptions for which a group subscription already
    /// exists.
    pub fn remove_single_subscriptions_for_group(
        &mut self,
        group_id: LargestGroupIdRaw,
        dest: impl SendEventProvider<Event, Error=E> + 'static
    ) {
        if self.listeners.contains_key(&ListenerType::Group(group_id)) {
            for (ltype, listeners) in &mut self.listeners {
                if let ListenerType::Single(_) = ltype {
                    listeners.retain(|f| {
                        f.send_provider.id() != dest.id()
                    });
                }
            }
        }
    }
}

impl<E, Provider: GenericEvent + Copy> EventManager<E, Provider> {
    fn update_listeners(
        &mut self,
        key: ListenerType,
        dest: impl SendEventProvider<Provider, Error = E> + 'static,
    ) {
        if !self.listeners.contains_key(&key) {
            self.listeners.insert(
                key,
                vec![Listener {
                    ltype: key,
                    send_provider: Box::new(dest),
                }],
            );
        } else {
            let vec = self.listeners.get_mut(&key).unwrap();
            // To prevent double insertions
            for entry in vec.iter() {
                if entry.ltype == key && entry.send_provider.id() == dest.id() {
                    return;
                }
            }
            vec.push(Listener {
                ltype: key,
                send_provider: Box::new(dest),
            });
        }
    }

    pub fn try_event_handling(&mut self) -> Result<HandlerResult<Provider>, E> {
        let mut err_status = None;
        let mut num_recipients = 0;
        let mut send_handler =
            |event: Provider, aux_data: Option<&[u8]>, llist: &mut Vec<Listener<E, Provider>>| {
                for listener in llist.iter_mut() {
                    if let Err(e) = listener.send_provider.send(event, aux_data) {
                        err_status = Some(Err(e));
                    } else {
                        num_recipients += 1;
                    }
                }
            };
        if let Some(event) = self
            .event_receiver
            .receive(self.aux_data_buf.as_mut_slice())
        {
            let single_key = ListenerType::Single(event.raw_as_largest_type());
            if self.listeners.contains_key(&single_key) {
                send_handler(
                    event,
                    Some(self.aux_data_buf.as_slice()),
                    self.listeners.get_mut(&single_key).unwrap(),
                );
            }
            let group_key = ListenerType::Group(event.group_id_as_largest_type());
            if self.listeners.contains_key(&group_key) {
                send_handler(
                    event,
                    Some(self.aux_data_buf.as_slice()),
                    self.listeners.get_mut(&group_key).unwrap(),
                );
            }
            if let Some(all_receivers) = self.listeners.get_mut(&ListenerType::All) {
                send_handler(event, Some(self.aux_data_buf.as_slice()), all_receivers);
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
    use super::{EventReceiver, HandlerResult, SendEventProvider};
    use crate::event_man::EventManager;
    use crate::events::{EventU32, GenericEvent, Severity};
    use alloc::boxed::Box;
    use std::sync::mpsc::{channel, Receiver, SendError, Sender};
    use std::time::Duration;
    use std::{thread, vec};
    use vec::Vec;

    type EventAndParams<'a> = (EventU32, Option<&'a [u8]>);

    struct MpscEventReceiver {
        mpsc_receiver: Receiver<(EventU32, Option<Vec<u8>>)>,
    }

    impl EventReceiver<EventU32> for MpscEventReceiver {
        fn receive(&mut self, aux_data: &mut [u8]) -> Option<EventU32> {
            if let Some((event, params)) = self.mpsc_receiver.try_recv().ok() {
                if let Some(params) = params {
                    if params.len() < aux_data.len() {
                        aux_data[0..params.len()].copy_from_slice(params.as_slice())
                    }
                }
                return Some(event);
            }
            None
        }
    }

    #[derive(Clone)]
    struct MpscEventSenderQueue<'a> {
        id: u32,
        mpsc_sender: Sender<EventAndParams<'a>>,
    }

    impl<'a> SendEventProvider<EventU32> for MpscEventSenderQueue<'a> {
        type Error = SendError<EventAndParams<'a>>;

        fn id(&self) -> u32 {
            self.id
        }
        fn send(&mut self, event: EventU32, _aux_data: Option<&[u8]>) -> Result<(), Self::Error> {
            self.mpsc_sender.send((event, None))
        }
    }

    fn check_next_event(expected: EventU32, receiver: &Receiver<EventAndParams>) {
        for _ in 0..5 {
            if let Ok(event) = receiver.try_recv() {
                assert_eq!(event.0, expected);
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
        let event_man_receiver = MpscEventReceiver {
            mpsc_receiver: manager_queue,
        };
        let mut event_man: EventManager<SendError<EventAndParams>, EventU32> =
            EventManager::new(Box::new(event_man_receiver), 128);
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
            .send((event_grp_0, None))
            .expect("Sending single error failed");
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_grp_0, 1);
        check_next_event(event_grp_0, &single_event_receiver);

        // Test event which is sent to all group listeners
        event_sender
            .send((event_grp_1_0, None))
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
        let event_man_receiver = MpscEventReceiver {
            mpsc_receiver: manager_queue,
        };
        let mut event_man: EventManager<SendError<EventAndParams>, EventU32> =
            EventManager::new(Box::new(event_man_receiver), 128);
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
            .send((event_grp_0, None))
            .expect("Sending Event Group 0 failed");
        event_sender
            .send((event_grp_1_0, None))
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
        let event_man_receiver = MpscEventReceiver {
            mpsc_receiver: manager_queue,
        };
        let mut event_man: EventManager<SendError<EventAndParams>, EventU32> =
            EventManager::new(Box::new(event_man_receiver), 128);
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
            .send((event_0, None))
            .expect("Triggering Event 0 failed");
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_0, 2);
        check_next_event(event_0, &event_0_rx_0);
        check_next_event(event_0, &event_0_rx_1);
        event_man.subscribe_group(event_1.group_id(), event_listener_0.clone());
        event_sender
            .send((event_0, None))
            .expect("Triggering Event 0 failed");
        event_sender
            .send((event_1, None))
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
            .send((event_1, None))
            .expect("Triggering Event 1 failed");
        let res = event_man.try_event_handling();
        assert!(res.is_ok());
        check_handled_event(res.unwrap(), event_1, 1);
    }
}
