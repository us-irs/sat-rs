use crate::events::{EventU32, GenericEvent, Severity};
#[cfg(feature = "alloc")]
use crate::events::{EventU32TypedSev, HasSeverity};
#[cfg(feature = "alloc")]
use alloc::boxed::Box;
#[cfg(feature = "alloc")]
use core::hash::Hash;
#[cfg(feature = "alloc")]
use hashbrown::HashSet;

#[cfg(feature = "alloc")]
pub use crate::pus::event::EventReporter;
use crate::pus::verification::{TcStateStarted, VerificationToken};
use crate::pus::EcssTmErrorWithSend;
#[cfg(feature = "alloc")]
use crate::pus::EcssTmSenderCore;
#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub use alloc_mod::*;
#[cfg(feature = "heapless")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "heapless")))]
pub use heapless_mod::*;

/// This trait allows the PUS event manager implementation to stay generic over various types
/// of backend containers.
///
/// These backend containers keep track on whether a particular event is enabled or disabled for
/// reporting and also expose a simple API to enable or disable the event reporting.
///
/// For example, a straight forward implementation for host systems could use a
/// [hash set](https://docs.rs/hashbrown/latest/hashbrown/struct.HashSet.html)
/// structure to track disabled events. A more primitive and embedded friendly
/// solution could track this information in a static or pre-allocated list which contains
/// the disabled events.
pub trait PusEventMgmtBackendProvider<Provider: GenericEvent> {
    type Error;

    fn event_enabled(&self, event: &Provider) -> bool;
    fn enable_event_reporting(&mut self, event: &Provider) -> Result<bool, Self::Error>;
    fn disable_event_reporting(&mut self, event: &Provider) -> Result<bool, Self::Error>;
}

#[cfg(feature = "heapless")]
pub mod heapless_mod {
    use super::*;
    use crate::events::{GenericEvent, LargestEventRaw};
    use std::marker::PhantomData;

    #[cfg_attr(doc_cfg, doc(cfg(feature = "heapless")))]
    // TODO: After a new version of heapless is released which uses hash32 version 0.3, try using
    //       regular Event type again.
    #[derive(Default)]
    pub struct HeaplessPusMgmtBackendProvider<const N: usize, Provider: GenericEvent> {
        disabled: heapless::FnvIndexSet<LargestEventRaw, N>,
        phantom: PhantomData<Provider>,
    }

    /// Safety: All contained field are [Send] as well
    unsafe impl<const N: usize, Event: GenericEvent + Send> Send
        for HeaplessPusMgmtBackendProvider<N, Event>
    {
    }

    impl<const N: usize, Provider: GenericEvent> PusEventMgmtBackendProvider<Provider>
        for HeaplessPusMgmtBackendProvider<N, Provider>
    {
        type Error = ();

        fn event_enabled(&self, event: &Provider) -> bool {
            self.disabled.contains(&event.raw_as_largest_type())
        }

        fn enable_event_reporting(&mut self, event: &Provider) -> Result<bool, Self::Error> {
            self.disabled
                .insert(event.raw_as_largest_type())
                .map_err(|_| ())
        }

        fn disable_event_reporting(&mut self, event: &Provider) -> Result<bool, Self::Error> {
            Ok(self.disabled.remove(&event.raw_as_largest_type()))
        }
    }
}

#[derive(Debug)]
pub enum EventRequest<Event: GenericEvent = EventU32> {
    Enable(Event),
    Disable(Event),
}

#[derive(Debug)]
pub struct EventRequestWithToken<Event: GenericEvent = EventU32> {
    pub request: EventRequest<Event>,
    pub token: VerificationToken<TcStateStarted>,
}

#[derive(Debug)]
pub enum EventManError<SenderE> {
    EcssTmError(EcssTmErrorWithSend<SenderE>),
    SeverityMissmatch(Severity, Severity),
}

impl<SenderE> From<EcssTmErrorWithSend<SenderE>> for EventManError<SenderE> {
    fn from(v: EcssTmErrorWithSend<SenderE>) -> Self {
        Self::EcssTmError(v)
    }
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use super::*;

    /// Default backend provider which uses a hash set as the event reporting status container
    /// like mentioned in the example of the [PusEventMgmtBackendProvider] documentation.
    ///
    /// This provider is a good option for host systems or larger embedded systems where
    /// the expected occasional memory allocation performed by the [HashSet] is not an issue.
    pub struct DefaultPusMgmtBackendProvider<Event: GenericEvent = EventU32> {
        disabled: HashSet<Event>,
    }

    /// Safety: All contained field are [Send] as well
    unsafe impl<Event: GenericEvent + Send> Send for DefaultPusMgmtBackendProvider<Event> {}

    impl<Event: GenericEvent> Default for DefaultPusMgmtBackendProvider<Event> {
        fn default() -> Self {
            Self {
                disabled: HashSet::default(),
            }
        }
    }

    impl<Provider: GenericEvent + PartialEq + Eq + Hash + Copy + Clone>
        PusEventMgmtBackendProvider<Provider> for DefaultPusMgmtBackendProvider<Provider>
    {
        type Error = ();
        fn event_enabled(&self, event: &Provider) -> bool {
            !self.disabled.contains(event)
        }

        fn enable_event_reporting(&mut self, event: &Provider) -> Result<bool, Self::Error> {
            Ok(self.disabled.remove(event))
        }

        fn disable_event_reporting(&mut self, event: &Provider) -> Result<bool, Self::Error> {
            Ok(self.disabled.insert(*event))
        }
    }

    pub struct PusEventDispatcher<BackendError, Provider: GenericEvent> {
        reporter: EventReporter,
        backend: Box<dyn PusEventMgmtBackendProvider<Provider, Error = BackendError>>,
    }

    /// Safety: All contained fields are send as well.
    unsafe impl<E: Send, Event: GenericEvent + Send> Send for PusEventDispatcher<E, Event> {}

    impl<BackendError, Provider: GenericEvent> PusEventDispatcher<BackendError, Provider> {
        pub fn new(
            reporter: EventReporter,
            backend: Box<dyn PusEventMgmtBackendProvider<Provider, Error = BackendError>>,
        ) -> Self {
            Self { reporter, backend }
        }
    }

    impl<BackendError, Event: GenericEvent> PusEventDispatcher<BackendError, Event> {
        pub fn enable_tm_for_event(&mut self, event: &Event) -> Result<bool, BackendError> {
            self.backend.enable_event_reporting(event)
        }

        pub fn disable_tm_for_event(&mut self, event: &Event) -> Result<bool, BackendError> {
            self.backend.disable_event_reporting(event)
        }

        pub fn generate_pus_event_tm_generic<E>(
            &mut self,
            sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
            time_stamp: &[u8],
            event: Event,
            aux_data: Option<&[u8]>,
        ) -> Result<bool, EventManError<E>> {
            if !self.backend.event_enabled(&event) {
                return Ok(false);
            }
            match event.severity() {
                Severity::INFO => self
                    .reporter
                    .event_info(sender, time_stamp, event, aux_data)
                    .map(|_| true)
                    .map_err(|e| e.into()),
                Severity::LOW => self
                    .reporter
                    .event_low_severity(sender, time_stamp, event, aux_data)
                    .map(|_| true)
                    .map_err(|e| e.into()),
                Severity::MEDIUM => self
                    .reporter
                    .event_medium_severity(sender, time_stamp, event, aux_data)
                    .map(|_| true)
                    .map_err(|e| e.into()),
                Severity::HIGH => self
                    .reporter
                    .event_high_severity(sender, time_stamp, event, aux_data)
                    .map(|_| true)
                    .map_err(|e| e.into()),
            }
        }
    }

    impl<BackendError> PusEventDispatcher<BackendError, EventU32> {
        pub fn enable_tm_for_event_with_sev<Severity: HasSeverity>(
            &mut self,
            event: &EventU32TypedSev<Severity>,
        ) -> Result<bool, BackendError> {
            self.backend.enable_event_reporting(event.as_ref())
        }

        pub fn disable_tm_for_event_with_sev<Severity: HasSeverity>(
            &mut self,
            event: &EventU32TypedSev<Severity>,
        ) -> Result<bool, BackendError> {
            self.backend.disable_event_reporting(event.as_ref())
        }

        pub fn generate_pus_event_tm<E, Severity: HasSeverity>(
            &mut self,
            sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
            time_stamp: &[u8],
            event: EventU32TypedSev<Severity>,
            aux_data: Option<&[u8]>,
        ) -> Result<bool, EventManError<E>> {
            self.generate_pus_event_tm_generic(sender, time_stamp, event.into(), aux_data)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::SeverityInfo;
    use spacepackets::tm::PusTm;
    use std::sync::mpsc::{channel, SendError, TryRecvError};
    use std::vec::Vec;

    const INFO_EVENT: EventU32TypedSev<SeverityInfo> =
        EventU32TypedSev::<SeverityInfo>::const_new(1, 0);
    const LOW_SEV_EVENT: EventU32 = EventU32::const_new(Severity::LOW, 1, 5);
    const EMPTY_STAMP: [u8; 7] = [0; 7];

    #[derive(Clone)]
    struct EventTmSender {
        sender: std::sync::mpsc::Sender<Vec<u8>>,
    }

    impl EcssTmSenderCore for EventTmSender {
        type Error = SendError<Vec<u8>>;
        fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmErrorWithSend<Self::Error>> {
            let mut vec = Vec::new();
            tm.append_to_vec(&mut vec)
                .map_err(|e| EcssTmErrorWithSend::EcssTmError(e.into()))?;
            self.sender
                .send(vec)
                .map_err(EcssTmErrorWithSend::SendError)?;
            Ok(())
        }
    }

    fn create_basic_man() -> PusEventDispatcher<(), EventU32> {
        let reporter = EventReporter::new(0x02, 128).expect("Creating event repoter failed");
        let backend = DefaultPusMgmtBackendProvider::<EventU32>::default();
        PusEventDispatcher::new(reporter, Box::new(backend))
    }

    #[test]
    fn test_basic() {
        let mut event_man = create_basic_man();
        let (event_tx, event_rx) = channel();
        let mut sender = EventTmSender { sender: event_tx };
        let event_sent = event_man
            .generate_pus_event_tm(&mut sender, &EMPTY_STAMP, INFO_EVENT, None)
            .expect("Sending info event failed");

        assert!(event_sent);
        // Will not check packet here, correctness of packet was tested somewhere else
        event_rx.try_recv().expect("Receiving event TM failed");
    }

    #[test]
    fn test_disable_event() {
        let mut event_man = create_basic_man();
        let (event_tx, event_rx) = channel();
        let mut sender = EventTmSender { sender: event_tx };
        let res = event_man.disable_tm_for_event(&LOW_SEV_EVENT);
        assert!(res.is_ok());
        assert!(res.unwrap());
        let mut event_sent = event_man
            .generate_pus_event_tm_generic(&mut sender, &EMPTY_STAMP, LOW_SEV_EVENT, None)
            .expect("Sending low severity event failed");
        assert!(!event_sent);
        let res = event_rx.try_recv();
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), TryRecvError::Empty));
        // Check that only the low severity event was disabled
        event_sent = event_man
            .generate_pus_event_tm(&mut sender, &EMPTY_STAMP, INFO_EVENT, None)
            .expect("Sending info event failed");
        assert!(event_sent);
        event_rx.try_recv().expect("No info event received");
    }

    #[test]
    fn test_reenable_event() {
        let mut event_man = create_basic_man();
        let (event_tx, event_rx) = channel();
        let mut sender = EventTmSender { sender: event_tx };
        let mut res = event_man.disable_tm_for_event_with_sev(&INFO_EVENT);
        assert!(res.is_ok());
        assert!(res.unwrap());
        res = event_man.enable_tm_for_event_with_sev(&INFO_EVENT);
        assert!(res.is_ok());
        assert!(res.unwrap());
        let event_sent = event_man
            .generate_pus_event_tm(&mut sender, &EMPTY_STAMP, INFO_EVENT, None)
            .expect("Sending info event failed");
        assert!(event_sent);
        event_rx.try_recv().expect("No info event received");
    }
}
