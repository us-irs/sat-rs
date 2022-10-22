use crate::events::Event;
use hashbrown::HashSet;

#[cfg(feature = "heapless")]
pub use heapless_mod::*;

/// This trait allows the PUS event manager implementation to stay generic over various types
/// of backend containers. These backend containers keep track on whether a particular event
/// is enabled or disabled for reporting and also expose a simple API to enable or disable the event
/// reporting.
///
/// For example, a straight forward implementation for host systems could use a
/// [hash set](https://docs.rs/hashbrown/latest/hashbrown/struct.HashSet.html)
/// structure to track disabled events. A more primitive and embedded friendly
/// solution could track this information in a static or pre-allocated list which contains
/// the disabled events.
pub trait PusEventMgmtBackendProvider {
    type Error;

    fn event_enabled(&self, event: &Event) -> bool;
    fn enable_event_reporting(&mut self, event: &Event) -> Result<bool, Self::Error>;
    fn disable_event_reporting(&mut self, event: &Event) -> Result<bool, Self::Error>;
}

/// Default backend provider which uses a hash set as the event reporting status container
/// like mentioned in the example of the [PusEventMgmtBackendProvider] documentation.
///
/// This provider is a good option for host systems or larger embedded systems where
/// the expected occasional memory allocation performed by the [HashSet] is not an issue.
#[derive(Default)]
pub struct DefaultPusMgmtBackendProvider {
    disabled: HashSet<Event>,
}

impl PusEventMgmtBackendProvider for DefaultPusMgmtBackendProvider {
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

#[cfg(feature = "heapless")]
pub mod heapless_mod {
    use super::*;
    use crate::events::EventRaw;

    // TODO: After a new version of heapless is released which uses hash32 version 0.3, try using
    //       regular Event type again.
    #[derive(Default)]
    pub struct HeaplessPusMgmtBckendProvider<const N: usize> {
        disabled: heapless::FnvIndexSet<EventRaw, N>,
    }

    impl<const N: usize> PusEventMgmtBackendProvider for HeaplessPusMgmtBckendProvider<N> {
        type Error = ();

        fn event_enabled(&self, event: &Event) -> bool {
            self.disabled.contains(&event.raw())
        }

        fn enable_event_reporting(&mut self, event: &Event) -> Result<bool, Self::Error> {
            self.disabled.insert(event.raw()).map_err(|_| ())
        }

        fn disable_event_reporting(&mut self, event: &Event) -> Result<bool, Self::Error> {
            Ok(self.disabled.remove(&event.raw()))
        }
    }
}

pub struct PusEventManager {}
