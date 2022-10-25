use crate::events::{EventU16TypedSev, EventU32TypedSev, GenericEvent, HasSeverity, Severity};
use alloc::boxed::Box;
use core::hash::Hash;
use hashbrown::HashSet;

use crate::pus::event::EventReporter;
use crate::pus::{EcssTmError, EcssTmSender};
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

/// Default backend provider which uses a hash set as the event reporting status container
/// like mentioned in the example of the [PusEventMgmtBackendProvider] documentation.
///
/// This provider is a good option for host systems or larger embedded systems where
/// the expected occasional memory allocation performed by the [HashSet] is not an issue.
#[derive(Default)]
pub struct DefaultPusMgmtBackendProvider<Provider: GenericEvent> {
    disabled: HashSet<Provider>,
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

#[cfg(feature = "heapless")]
pub mod heapless_mod {
    use super::*;
    use crate::events::{GenericEvent, LargestEventRaw};
    use std::marker::PhantomData;

    #[cfg_attr(doc_cfg, doc(cfg(feature = "heapless")))]
    // TODO: After a new version of heapless is released which uses hash32 version 0.3, try using
    //       regular Event type again.
    #[derive(Default)]
    pub struct HeaplessPusMgmtBckendProvider<const N: usize, Provider: GenericEvent> {
        disabled: heapless::FnvIndexSet<LargestEventRaw, N>,
        phantom: PhantomData<Provider>,
    }

    impl<const N: usize, Provider: GenericEvent> PusEventMgmtBackendProvider<Provider>
        for HeaplessPusMgmtBckendProvider<N, Provider>
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

pub struct PusEventManager<BackendError, Provider: GenericEvent> {
    reporter: EventReporter,
    backend: Box<dyn PusEventMgmtBackendProvider<Provider, Error = BackendError>>,
}

impl<BackendError, Event: GenericEvent> PusEventManager<BackendError, Event> {
    pub fn enable_tm_for_event(&mut self, event: &Event) -> Result<bool, BackendError> {
        self.backend.enable_event_reporting(event)
    }

    pub fn disable_tm_for_event(&mut self, event: &Event) -> Result<bool, BackendError> {
        self.backend.disable_event_reporting(event)
    }

    pub fn generate_pus_event_tm_generic<E>(
        &mut self,
        severity: Severity,
        sender: &mut (impl EcssTmSender<E> + ?Sized),
        time_stamp: &[u8],
        event: Event,
        aux_data: Option<&[u8]>,
    ) -> Result<bool, EcssTmError<E>> {
        if !self.backend.event_enabled(&event) {
            return Ok(false);
        }
        match severity {
            Severity::INFO => self
                .reporter
                .event_info(sender, time_stamp, event, aux_data)
                .map(|_| true),
            Severity::LOW => self
                .reporter
                .event_low_severity(sender, time_stamp, event, aux_data)
                .map(|_| true),
            Severity::MEDIUM => self
                .reporter
                .event_medium_severity(sender, time_stamp, event, aux_data)
                .map(|_| true),
            Severity::HIGH => self
                .reporter
                .event_high_severity(sender, time_stamp, event, aux_data)
                .map(|_| true),
        }
    }
}

impl<BackendError, SEVERITY: HasSeverity>
    PusEventManager<BackendError, EventU32TypedSev<SEVERITY>>
{
    pub fn generate_pus_event_tm<E>(
        &mut self,
        sender: &mut (impl EcssTmSender<E> + ?Sized),
        time_stamp: &[u8],
        event: EventU32TypedSev<SEVERITY>,
        aux_data: Option<&[u8]>,
    ) -> Result<bool, EcssTmError<E>> {
        self.generate_pus_event_tm_generic(SEVERITY::SEVERITY, sender, time_stamp, event, aux_data)
    }
}

impl<BackendError, SEVERITY: HasSeverity>
    PusEventManager<BackendError, EventU16TypedSev<SEVERITY>>
{
    pub fn generate_pus_event_tm<E>(
        &mut self,
        sender: &mut (impl EcssTmSender<E> + ?Sized),
        time_stamp: &[u8],
        event: EventU16TypedSev<SEVERITY>,
        aux_data: Option<&[u8]>,
    ) -> Result<bool, EcssTmError<E>> {
        self.generate_pus_event_tm_generic(SEVERITY::SEVERITY, sender, time_stamp, event, aux_data)
    }
}
