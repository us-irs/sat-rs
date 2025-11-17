//! # Event support module
//!
//! The user can define events as custom structs or enumerations.
//! The event structures defined here are type erased and rely on some properties which
//! should be provided by the user through the [Event] and [serde::Serialize] trait.
//!
//! This in turn allows to use higher-level abstractions like the event manger.
//!
//! This module includes the basic type erased event structs [EventErasedAlloc] and
//! [EventErasedHeapless].
//! The abstraction also allows to group related events using a group ID, and the severity
//! of an event is encoded inside the raw value itself with four possible [Severity] levels:
//!
//!  - INFO
//!  - LOW
//!  - MEDIUM
//!  - HIGH
use core::fmt::Debug;
use core::hash::Hash;

use arbitrary_int::{prelude::*, u14};
#[cfg(feature = "heapless")]
use spacepackets::ByteConversionError;

/// Using a type definition allows to change this to u64 in the future more easily
pub type LargestEventRaw = u32;
/// Using a type definition allows to change this to u32 in the future more easily
pub type LargestGroupIdRaw = u16;

pub const MAX_GROUP_ID_U32_EVENT: u16 = u14::MAX.value();

#[derive(
    Copy, Clone, PartialEq, Eq, Debug, Hash, num_enum::TryFromPrimitive, num_enum::IntoPrimitive,
)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum Severity {
    Info = 0,
    Low = 1,
    Medium = 2,
    High = 3,
}

pub trait HasSeverity: Debug + PartialEq + Eq + Copy + Clone {
    const SEVERITY: Severity;
}

pub trait Event: Clone {
    fn id(&self) -> EventId;
}

pub type GroupId = u14;

/// Unique event identifier.
///
/// Consists of a group ID, a unique ID and the severity.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct EventId {
    group_id: GroupId,
    unique_id: u16,
    severity: Severity,
}

impl EventId {
    pub fn new(severity: Severity, group_id: u14, unique_id: u16) -> Self {
        Self {
            severity,
            group_id,
            unique_id,
        }
    }

    #[inline]
    pub fn unique_id(&self) -> u16 {
        self.unique_id
    }

    #[inline]
    pub fn severity(&self) -> Severity {
        self.severity
    }

    #[inline]
    pub fn group_id(&self) -> u14 {
        self.group_id
    }

    pub fn raw(&self) -> u32 {
        ((self.severity as u32) << 30)
            | ((self.group_id.as_u16() as u32) << 16)
            | (self.unique_id as u32)
    }
}

impl From<u32> for EventId {
    fn from(raw: u32) -> Self {
        // Severity conversion from u8 should never fail
        let severity = Severity::try_from(((raw >> 30) & 0b11) as u8).unwrap();
        let group_id = u14::new(((raw >> 16) & 0x3FFF) as u16);
        let unique_id = (raw & 0xFFFF) as u16;
        // Sanitized input, should never fail
        Self::new(severity, group_id, unique_id)
    }
}

/// Event which was type erased and serialized into a [alloc::vec::Vec].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct EventErasedAlloc {
    id: EventId,
    event_raw: alloc::vec::Vec<u8>,
}

#[cfg(feature = "alloc")]
impl EventErasedAlloc {
    #[cfg(feature = "serde")]
    /// Creates a new event by serializing the given event using [postcard].
    pub fn new(event: &(impl serde::Serialize + Event)) -> Self {
        Self {
            id: event.id(),
            event_raw: postcard::to_allocvec(event).unwrap(),
        }
    }

    pub fn new_with_raw_event(id: EventId, event_raw: &[u8]) -> Self {
        Self {
            id,
            event_raw: event_raw.to_vec(),
        }
    }

    #[inline]
    pub fn raw(&self) -> &[u8] {
        &self.event_raw
    }
}

#[cfg(feature = "serde")]
#[cfg(feature = "alloc")]
impl<T: serde::Serialize + Event> From<T> for EventErasedAlloc {
    fn from(event: T) -> Self {
        Self::new(&event)
    }
}

#[cfg(feature = "alloc")]
impl Event for EventErasedAlloc {
    fn id(&self) -> EventId {
        self.id
    }
}

/// Event which was type erased and serialized into a [heapless::vec::Vec].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "heapless")]
pub struct EventErasedHeapless<const N: usize> {
    id: EventId,
    event_raw: heapless::vec::Vec<u8, N>,
}

#[cfg(feature = "heapless")]
impl<const N: usize> Event for EventErasedHeapless<N> {
    fn id(&self) -> EventId {
        self.id
    }
}

#[cfg(feature = "heapless")]
impl<const N: usize> EventErasedHeapless<N> {
    #[cfg(feature = "serde")]
    /// Creates a new event by serializing the given event using [postcard].
    pub fn new(event: &(impl serde::Serialize + Event)) -> Result<Self, ByteConversionError> {
        let ser_size = postcard::experimental::serialized_size(event).unwrap();
        if ser_size > N {
            return Err(ByteConversionError::ToSliceTooSmall {
                found: N,
                expected: ser_size,
            });
        }
        let mut vec = heapless::Vec::<u8, N>::new();
        vec.resize(N, 0).unwrap();
        postcard::to_slice(event, vec.as_mut_slice()).unwrap();
        Ok(Self {
            id: event.id(),
            event_raw: vec,
        })
    }

    pub fn new_with_raw_event(id: EventId, event_raw: heapless::Vec<u8, N>) -> Self {
        Self { id, event_raw }
    }

    #[inline]
    pub fn raw(&self) -> &[u8] {
        &self.event_raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub enum TestEvent {
        Info,
        ErrorOtherGroup,
    }

    impl Event for TestEvent {
        fn id(&self) -> EventId {
            match self {
                TestEvent::Info => EventId::new(Severity::Info, u14::new(0), 0),
                TestEvent::ErrorOtherGroup => EventId::new(Severity::High, u14::new(1), 1),
            }
        }
    }

    #[test]
    fn test_normal_event_getters() {
        assert_eq!(TestEvent::Info.id().severity(), Severity::Info);
        assert_eq!(TestEvent::Info.id().unique_id(), 0);
        assert_eq!(TestEvent::Info.id().group_id().value(), 0);
        assert_eq!(TestEvent::ErrorOtherGroup.id().group_id().value(), 1);
        assert_eq!(TestEvent::ErrorOtherGroup.id().unique_id(), 1);
        let raw_event = TestEvent::Info.id().raw();
        assert_eq!(raw_event, 0x00000000);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_basic_erased_alloc_event() {
        let event = EventErasedAlloc::new(&TestEvent::Info);
        let test_event: TestEvent = postcard::from_bytes(event.raw()).unwrap();
        assert_eq!(test_event, TestEvent::Info);
    }

    #[test]
    #[cfg(all(feature = "serde", feature = "alloc"))]
    fn test_basic_erased_heapless_event() {
        let event = EventErasedHeapless::<8>::new(&TestEvent::Info).unwrap();
        let test_event: TestEvent = postcard::from_bytes(event.raw()).unwrap();
        assert_eq!(test_event, TestEvent::Info);
    }
}
