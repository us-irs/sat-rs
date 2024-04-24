use crate::events::{EventU32, GenericEvent, Severity};
#[cfg(feature = "alloc")]
use crate::events::{EventU32TypedSev, HasSeverity};
#[cfg(feature = "alloc")]
use core::hash::Hash;
#[cfg(feature = "alloc")]
use hashbrown::HashSet;

#[cfg(feature = "alloc")]
pub use crate::pus::event::EventReporter;
use crate::pus::verification::TcStateToken;
#[cfg(feature = "alloc")]
use crate::pus::EcssTmSender;
use crate::pus::EcssTmtcError;
#[cfg(feature = "alloc")]
pub use alloc_mod::*;
#[cfg(feature = "heapless")]
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
pub trait PusEventReportingMapProvider<Event: GenericEvent> {
    type Error;

    fn event_enabled(&self, event: &Event) -> bool;
    fn enable_event_reporting(&mut self, event: &Event) -> Result<bool, Self::Error>;
    fn disable_event_reporting(&mut self, event: &Event) -> Result<bool, Self::Error>;
}

#[cfg(feature = "heapless")]
pub mod heapless_mod {
    use super::*;
    use crate::events::LargestEventRaw;
    use core::marker::PhantomData;

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

    impl<const N: usize, Provider: GenericEvent> PusEventReportingMapProvider<Provider>
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

#[derive(Debug, PartialEq, Eq)]
pub enum EventRequest<Event: GenericEvent = EventU32> {
    Enable(Event),
    Disable(Event),
}

#[derive(Debug)]
pub struct EventRequestWithToken<Event: GenericEvent = EventU32> {
    pub request: EventRequest<Event>,
    pub token: TcStateToken,
}

#[derive(Debug)]
pub enum EventManError {
    EcssTmtcError(EcssTmtcError),
    SeverityMissmatch(Severity, Severity),
}

impl From<EcssTmtcError> for EventManError {
    fn from(v: EcssTmtcError) -> Self {
        Self::EcssTmtcError(v)
    }
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use core::marker::PhantomData;

    use crate::{
        events::EventU16,
        pus::event::{DummyEventHook, EventTmHookProvider},
    };

    use super::*;

    /// Default backend provider which uses a hash set as the event reporting status container
    /// like mentioned in the example of the [PusEventReportingMapProvider] documentation.
    ///
    /// This provider is a good option for host systems or larger embedded systems where
    /// the expected occasional memory allocation performed by the [HashSet] is not an issue.
    pub struct DefaultPusEventReportingMap<Event: GenericEvent = EventU32> {
        disabled: HashSet<Event>,
    }

    impl<Event: GenericEvent> Default for DefaultPusEventReportingMap<Event> {
        fn default() -> Self {
            Self {
                disabled: HashSet::default(),
            }
        }
    }

    impl<Event: GenericEvent + PartialEq + Eq + Hash + Copy + Clone>
        PusEventReportingMapProvider<Event> for DefaultPusEventReportingMap<Event>
    {
        type Error = ();

        fn event_enabled(&self, event: &Event) -> bool {
            !self.disabled.contains(event)
        }

        fn enable_event_reporting(&mut self, event: &Event) -> Result<bool, Self::Error> {
            Ok(self.disabled.remove(event))
        }

        fn disable_event_reporting(&mut self, event: &Event) -> Result<bool, Self::Error> {
            Ok(self.disabled.insert(*event))
        }
    }

    pub struct PusEventTmCreatorWithMap<
        ReportingMap: PusEventReportingMapProvider<Event>,
        Event: GenericEvent,
        EventTmHook: EventTmHookProvider = DummyEventHook,
    > {
        pub reporter: EventReporter<EventTmHook>,
        reporting_map: ReportingMap,
        phantom: PhantomData<Event>,
    }

    impl<
            ReportingMap: PusEventReportingMapProvider<Event>,
            Event: GenericEvent,
            EventTmHook: EventTmHookProvider,
        > PusEventTmCreatorWithMap<ReportingMap, Event, EventTmHook>
    {
        pub fn new(reporter: EventReporter<EventTmHook>, backend: ReportingMap) -> Self {
            Self {
                reporter,
                reporting_map: backend,
                phantom: PhantomData,
            }
        }

        pub fn enable_tm_for_event(&mut self, event: &Event) -> Result<bool, ReportingMap::Error> {
            self.reporting_map.enable_event_reporting(event)
        }

        pub fn disable_tm_for_event(&mut self, event: &Event) -> Result<bool, ReportingMap::Error> {
            self.reporting_map.disable_event_reporting(event)
        }

        pub fn generate_pus_event_tm_generic(
            &self,
            sender: &(impl EcssTmSender + ?Sized),
            time_stamp: &[u8],
            event: Event,
            params: Option<&[u8]>,
        ) -> Result<bool, EventManError> {
            if !self.reporting_map.event_enabled(&event) {
                return Ok(false);
            }
            match event.severity() {
                Severity::INFO => self
                    .reporter
                    .event_info(sender, time_stamp, event, params)
                    .map(|_| true)
                    .map_err(|e| e.into()),
                Severity::LOW => self
                    .reporter
                    .event_low_severity(sender, time_stamp, event, params)
                    .map(|_| true)
                    .map_err(|e| e.into()),
                Severity::MEDIUM => self
                    .reporter
                    .event_medium_severity(sender, time_stamp, event, params)
                    .map(|_| true)
                    .map_err(|e| e.into()),
                Severity::HIGH => self
                    .reporter
                    .event_high_severity(sender, time_stamp, event, params)
                    .map(|_| true)
                    .map_err(|e| e.into()),
            }
        }
    }

    impl<Event: GenericEvent + Copy + PartialEq + Eq + Hash, EventTmHook: EventTmHookProvider>
        PusEventTmCreatorWithMap<DefaultPusEventReportingMap<Event>, Event, EventTmHook>
    {
        pub fn new_with_default_backend(reporter: EventReporter<EventTmHook>) -> Self {
            Self {
                reporter,
                reporting_map: DefaultPusEventReportingMap::default(),
                phantom: PhantomData,
            }
        }
    }

    impl<ReportingMap: PusEventReportingMapProvider<EventU32>>
        PusEventTmCreatorWithMap<ReportingMap, EventU32>
    {
        pub fn enable_tm_for_event_with_sev<Severity: HasSeverity>(
            &mut self,
            event: &EventU32TypedSev<Severity>,
        ) -> Result<bool, ReportingMap::Error> {
            self.reporting_map.enable_event_reporting(event.as_ref())
        }

        pub fn disable_tm_for_event_with_sev<Severity: HasSeverity>(
            &mut self,
            event: &EventU32TypedSev<Severity>,
        ) -> Result<bool, ReportingMap::Error> {
            self.reporting_map.disable_event_reporting(event.as_ref())
        }

        pub fn generate_pus_event_tm<Severity: HasSeverity>(
            &self,
            sender: &(impl EcssTmSender + ?Sized),
            time_stamp: &[u8],
            event: EventU32TypedSev<Severity>,
            aux_data: Option<&[u8]>,
        ) -> Result<bool, EventManError> {
            self.generate_pus_event_tm_generic(sender, time_stamp, event.into(), aux_data)
        }
    }

    pub type DefaultPusEventU16TmCreator<EventTmHook = DummyEventHook> =
        PusEventTmCreatorWithMap<DefaultPusEventReportingMap<EventU16>, EventU16, EventTmHook>;
    pub type DefaultPusEventU32TmCreator<EventTmHook = DummyEventHook> =
        PusEventTmCreatorWithMap<DefaultPusEventReportingMap<EventU32>, EventU32, EventTmHook>;
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::UniqueApidTargetId;
    use crate::{events::SeverityInfo, tmtc::PacketAsVec};
    use std::sync::mpsc::{self, TryRecvError};

    const INFO_EVENT: EventU32TypedSev<SeverityInfo> =
        EventU32TypedSev::<SeverityInfo>::const_new(1, 0);
    const LOW_SEV_EVENT: EventU32 = EventU32::const_new(Severity::LOW, 1, 5);
    const EMPTY_STAMP: [u8; 7] = [0; 7];
    const TEST_APID: u16 = 0x02;
    const TEST_ID: UniqueApidTargetId = UniqueApidTargetId::new(TEST_APID, 0x05);

    fn create_basic_man_1() -> DefaultPusEventU32TmCreator {
        let reporter = EventReporter::new(TEST_ID.raw(), TEST_APID, 0, 128)
            .expect("Creating event repoter failed");
        PusEventTmCreatorWithMap::new_with_default_backend(reporter)
    }
    fn create_basic_man_2() -> DefaultPusEventU32TmCreator {
        let reporter = EventReporter::new(TEST_ID.raw(), TEST_APID, 0, 128)
            .expect("Creating event repoter failed");
        let backend = DefaultPusEventReportingMap::default();
        PusEventTmCreatorWithMap::new(reporter, backend)
    }

    #[test]
    fn test_basic() {
        let event_man = create_basic_man_1();
        let (event_tx, event_rx) = mpsc::channel::<PacketAsVec>();
        let event_sent = event_man
            .generate_pus_event_tm(&event_tx, &EMPTY_STAMP, INFO_EVENT, None)
            .expect("Sending info event failed");

        assert!(event_sent);
        // Will not check packet here, correctness of packet was tested somewhere else
        event_rx.try_recv().expect("Receiving event TM failed");
    }

    #[test]
    fn test_disable_event() {
        let mut event_man = create_basic_man_2();
        let (event_tx, event_rx) = mpsc::channel::<PacketAsVec>();
        // let mut sender = TmAsVecSenderWithMpsc::new(0, "test", event_tx);
        let res = event_man.disable_tm_for_event(&LOW_SEV_EVENT);
        assert!(res.is_ok());
        assert!(res.unwrap());
        let mut event_sent = event_man
            .generate_pus_event_tm_generic(&event_tx, &EMPTY_STAMP, LOW_SEV_EVENT, None)
            .expect("Sending low severity event failed");
        assert!(!event_sent);
        let res = event_rx.try_recv();
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), TryRecvError::Empty));
        // Check that only the low severity event was disabled
        event_sent = event_man
            .generate_pus_event_tm(&event_tx, &EMPTY_STAMP, INFO_EVENT, None)
            .expect("Sending info event failed");
        assert!(event_sent);
        event_rx.try_recv().expect("No info event received");
    }

    #[test]
    fn test_reenable_event() {
        let mut event_man = create_basic_man_1();
        let (event_tx, event_rx) = mpsc::channel::<PacketAsVec>();
        let mut res = event_man.disable_tm_for_event_with_sev(&INFO_EVENT);
        assert!(res.is_ok());
        assert!(res.unwrap());
        res = event_man.enable_tm_for_event_with_sev(&INFO_EVENT);
        assert!(res.is_ok());
        assert!(res.unwrap());
        let event_sent = event_man
            .generate_pus_event_tm(&event_tx, &EMPTY_STAMP, INFO_EVENT, None)
            .expect("Sending info event failed");
        assert!(event_sent);
        event_rx.try_recv().expect("No info event received");
    }
}
