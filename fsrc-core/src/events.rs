//! Event support module

use core::hash::Hash;
use spacepackets::ecss::{EcssEnumeration, ToBeBytes};
use spacepackets::{ByteConversionError, SizeMissmatch};
use std::marker::PhantomData;

/// Using a type definition allows to change this to u64 in the future more easily
pub type LargestEventRaw = u32;
/// Using a type definition allows to change this to u32 in the future more easily
pub type LargestGroupIdRaw = u16;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum Severity {
    INFO = 0,
    LOW = 1,
    MEDIUM = 2,
    HIGH = 3,
}

pub trait EventProvider: PartialEq + Eq + Copy + Clone + Hash {
    type Raw;
    type GroupId;
    type UniqueId;

    fn raw(&self) -> Self::Raw;
    fn severity(&self) -> Severity;
    fn group_id(&self) -> Self::GroupId;
    fn unique_id(&self) -> Self::UniqueId;

    fn raw_as_largest_type(&self) -> LargestEventRaw;
    fn group_id_as_largest_type(&self) -> LargestGroupIdRaw;
}

impl TryFrom<u8> for Severity {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            x if x == Severity::INFO as u8 => Ok(Severity::INFO),
            x if x == Severity::LOW as u8 => Ok(Severity::LOW),
            x if x == Severity::MEDIUM as u8 => Ok(Severity::MEDIUM),
            x if x == Severity::HIGH as u8 => Ok(Severity::HIGH),
            _ => Err(()),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventBase<RAW, GID, UID> {
    severity: Severity,
    group_id: GID,
    unique_id: UID,
    phantom: PhantomData<RAW>,
}

impl<RAW: ToBeBytes, GID, UID> EventBase<RAW, GID, UID> {
    fn write_to_bytes(
        &self,
        raw: RAW,
        buf: &mut [u8],
        width: usize,
    ) -> Result<(), ByteConversionError> {
        if buf.len() < width {
            return Err(ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                found: buf.len(),
                expected: width,
            }));
        }
        buf.copy_from_slice(raw.to_be_bytes().as_ref());
        Ok(())
    }
}

impl EventBase<u32, u16, u16> {
    #[inline]
    fn raw(&self) -> u32 {
        (((self.severity as u32) << 30) | ((self.group_id as u32) << 16) | self.unique_id as u32)
            as u32
    }
}

impl EventBase<u16, u8, u8> {
    #[inline]
    fn raw(&self) -> u16 {
        (((self.severity as u16) << 14) as u16
            | ((self.group_id as u16) << 8) as u16
            | self.unique_id as u16) as u16
    }
}

impl<RAW, GID, UID> EventBase<RAW, GID, UID> {
    #[inline]
    pub fn severity(&self) -> Severity {
        self.severity
    }
}

impl<RAW, GID> EventBase<RAW, GID, u16> {
    #[inline]
    pub fn unique_id(&self) -> u16 {
        self.unique_id
    }
}

impl<RAW, GID> EventBase<RAW, GID, u8> {
    #[inline]
    pub fn unique_id(&self) -> u8 {
        self.unique_id
    }
}

impl<RAW, UID> EventBase<RAW, u16, UID> {
    #[inline]
    pub fn group_id(&self) -> u16 {
        self.group_id
    }
}

impl<RAW, UID> EventBase<RAW, u8, UID> {
    #[inline]
    pub fn group_id(&self) -> u8 {
        self.group_id
    }
}

macro_rules! event_provider_impl {
    () => {
        #[inline]
        fn raw(&self) -> Self::Raw {
            self.base.raw()
        }

        /// Retrieve the severity of an event. Returns None if that severity bit field of the raw event
        /// ID is invalid
        #[inline]
        fn severity(&self) -> Severity {
            self.base.severity()
        }

        #[inline]
        fn group_id(&self) -> Self::GroupId {
            self.base.group_id()
        }

        #[inline]
        fn unique_id(&self) -> Self::UniqueId {
            self.base.unique_id()
        }
    };
}
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Event {
    base: EventBase<u32, u16, u16>,
}

impl EventProvider for Event {
    type Raw = u32;
    type GroupId = u16;
    type UniqueId = u16;

    event_provider_impl!();

    fn raw_as_largest_type(&self) -> LargestEventRaw {
        self.raw()
    }

    fn group_id_as_largest_type(&self) -> LargestGroupIdRaw {
        self.group_id()
    }
}

impl Event {
    /// Generate an event. The raw representation of an event has 32 bits.
    /// If the passed group ID is invalid (too large), None wil be returned
    ///
    /// # Parameter
    ///
    /// * `severity`: Each event has a [severity][Severity]. The raw value of the severity will
    ///        be stored inside the uppermost 2 bits of the raw event ID
    /// * `group_id`: Related events can be grouped using a group ID. The group ID will occupy the
    ///        next 14 bits after the severity. Therefore, the size is limited by dec 16383 hex 0x3FFF.
    /// * `unique_id`: Each event has a unique 16 bit ID occupying the last 16 bits of the
    ///       raw event ID
    pub fn new(
        severity: Severity,
        group_id: <Self as EventProvider>::GroupId,
        unique_id: <Self as EventProvider>::UniqueId,
    ) -> Option<Self> {
        if group_id > (2u16.pow(14) - 1) {
            return None;
        }
        Some(Self {
            base: EventBase {
                severity,
                group_id,
                unique_id,
                phantom: PhantomData,
            },
        })
    }

    /// Const version of [new], but panics on invalid group ID input values.
    pub const fn const_new(
        severity: Severity,
        group_id: <Self as EventProvider>::GroupId,
        unique_id: <Self as EventProvider>::UniqueId,
    ) -> Self {
        if group_id > (2u16.pow(14) - 1) {
            panic!("Group ID too large");
        }
        Self {
            base: EventBase {
                severity,
                group_id,
                unique_id,
                phantom: PhantomData,
            },
        }
    }
}

impl From<u32> for Event {
    fn from(raw: u32) -> Self {
        // Severity conversion from u8 should never fail
        let severity = Severity::try_from(((raw >> 30) & 0b11) as u8).unwrap();
        let group_id = ((raw >> 16) & 0x3FFF) as u16;
        let unique_id = (raw & 0xFFFF) as u16;
        // Sanitized input, should never fail
        Self::const_new(severity, group_id, unique_id)
    }
}

impl EcssEnumeration for Event {
    fn pfc(&self) -> u8 {
        32
    }

    fn write_to_bytes(&self, buf: &mut [u8]) -> Result<(), ByteConversionError> {
        self.base.write_to_bytes(self.raw(), buf, self.byte_width())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventSmall {
    base: EventBase<u16, u8, u8>,
}

impl EventSmall {
    /// Generate a small event. The raw representation of a small event has 16 bits.
    /// If the passed group ID is invalid (too large), [None] wil be returned
    ///
    /// # Parameter
    ///
    /// * `severity`: Each event has a [severity][Severity]. The raw value of the severity will
    ///        be stored inside the uppermost 2 bits of the raw event ID
    /// * `group_id`: Related events can be grouped using a group ID. The group ID will occupy the
    ///        next 6 bits after the severity. Therefore, the size is limited by dec 63 hex 0x3F.
    /// * `unique_id`: Each event has a unique 8 bit ID occupying the last 8 bits of the
    ///       raw event ID
    pub fn new(
        severity: Severity,
        group_id: <Self as EventProvider>::GroupId,
        unique_id: <Self as EventProvider>::UniqueId,
    ) -> Option<Self> {
        if group_id > (2u8.pow(6) - 1) {
            return None;
        }
        Some(Self {
            base: EventBase {
                severity,
                group_id,
                unique_id,
                phantom: Default::default(),
            },
        })
    }

    pub const fn const_new(
        severity: Severity,
        group_id: <Self as EventProvider>::GroupId,
        unique_id: <Self as EventProvider>::UniqueId,
    ) -> Self {
        if group_id > (2u8.pow(6) - 1) {
            panic!("Group ID too large");
        }
        Self {
            base: EventBase {
                severity,
                group_id,
                unique_id,
                phantom: PhantomData,
            },
        }
    }
}

impl EventProvider for EventSmall {
    type Raw = u16;
    type GroupId = u8;
    type UniqueId = u8;

    event_provider_impl!();

    fn raw_as_largest_type(&self) -> LargestEventRaw {
        self.raw().into()
    }

    fn group_id_as_largest_type(&self) -> LargestGroupIdRaw {
        self.group_id().into()
    }
}

impl EcssEnumeration for EventSmall {
    fn pfc(&self) -> u8 {
        16
    }

    fn write_to_bytes(&self, buf: &mut [u8]) -> Result<(), ByteConversionError> {
        self.base.write_to_bytes(self.raw(), buf, self.byte_width())
    }
}

impl From<u16> for EventSmall {
    fn from(raw: <Self as EventProvider>::Raw) -> Self {
        let severity = Severity::try_from(((raw >> 14) & 0b11) as u8).unwrap();
        let group_id = ((raw >> 8) & 0x3F) as u8;
        let unique_id = (raw & 0xFF) as u8;
        // Sanitized input, new call should never fail
        Self::const_new(severity, group_id, unique_id)
    }
}

#[cfg(test)]
mod tests {
    use super::Event;
    use crate::events::{EventProvider, EventSmall, Severity};
    use spacepackets::ecss::EcssEnumeration;
    use spacepackets::ByteConversionError;
    use std::mem::size_of;

    fn assert_size<T>(_: T, val: usize) {
        assert_eq!(size_of::<T>(), val);
    }

    const INFO_EVENT: Event = Event::const_new(Severity::INFO, 0, 0);
    const INFO_EVENT_SMALL: EventSmall = EventSmall::const_new(Severity::INFO, 0, 0);
    const HIGH_SEV_EVENT: Event = Event::const_new(Severity::HIGH, 0x3FFF, 0xFFFF);
    const HIGH_SEV_EVENT_SMALL: EventSmall = EventSmall::const_new(Severity::HIGH, 0x3F, 0xff);

    #[test]
    fn test_normal_from_raw_conversion() {
        let conv_from_raw = Event::from(INFO_EVENT.raw());
        assert_eq!(conv_from_raw, INFO_EVENT);
    }

    #[test]
    fn test_small_from_raw_conversion() {
        let conv_from_raw = EventSmall::from(INFO_EVENT_SMALL.raw());
        assert_eq!(conv_from_raw, INFO_EVENT_SMALL);
    }

    #[test]
    fn verify_normal_size() {
        assert_size(INFO_EVENT.raw(), 4)
    }

    #[test]
    fn verify_small_size() {
        assert_size(INFO_EVENT_SMALL.raw(), 2)
    }

    #[test]
    fn test_normal_event_getters() {
        assert_eq!(INFO_EVENT.severity(), Severity::INFO);
        assert_eq!(INFO_EVENT.unique_id(), 0);
        assert_eq!(INFO_EVENT.group_id(), 0);
        let raw_event = INFO_EVENT.raw();
        assert_eq!(raw_event, 0x00000000);
    }

    #[test]
    fn test_small_event_getters() {
        assert_eq!(INFO_EVENT_SMALL.severity(), Severity::INFO);
        assert_eq!(INFO_EVENT_SMALL.unique_id(), 0);
        assert_eq!(INFO_EVENT_SMALL.group_id(), 0);
        let raw_event = INFO_EVENT_SMALL.raw();
        assert_eq!(raw_event, 0x00000000);
    }

    #[test]
    fn all_ones_event_regular() {
        assert_eq!(HIGH_SEV_EVENT.severity(), Severity::HIGH);
        assert_eq!(HIGH_SEV_EVENT.group_id(), 0x3FFF);
        assert_eq!(HIGH_SEV_EVENT.unique_id(), 0xFFFF);
        let raw_event = HIGH_SEV_EVENT.raw();
        assert_eq!(raw_event, 0xFFFFFFFF);
    }

    #[test]
    fn all_ones_event_small() {
        assert_eq!(HIGH_SEV_EVENT_SMALL.severity(), Severity::HIGH);
        assert_eq!(HIGH_SEV_EVENT_SMALL.group_id(), 0x3F);
        assert_eq!(HIGH_SEV_EVENT_SMALL.unique_id(), 0xFF);
        let raw_event = HIGH_SEV_EVENT_SMALL.raw();
        assert_eq!(raw_event, 0xFFFF);
    }

    #[test]
    fn invalid_group_id_normal() {
        assert!(Event::new(Severity::MEDIUM, 2_u16.pow(14), 0).is_none());
    }

    #[test]
    fn invalid_group_id_small() {
        assert!(EventSmall::new(Severity::MEDIUM, 2_u8.pow(6), 0).is_none());
    }

    #[test]
    fn regular_new() {
        assert_eq!(
            Event::new(Severity::INFO, 0, 0).expect("Creating regular event failed"),
            INFO_EVENT
        );
    }

    #[test]
    fn small_new() {
        assert_eq!(
            EventSmall::new(Severity::INFO, 0, 0).expect("Creating regular event failed"),
            INFO_EVENT_SMALL
        );
    }

    #[test]
    fn as_largest_type() {
        let event_raw = HIGH_SEV_EVENT.raw_as_largest_type();
        assert_size(event_raw, 4);
        assert_eq!(event_raw, 0xFFFFFFFF);
    }

    #[test]
    fn as_largest_type_for_small_event() {
        let event_raw = HIGH_SEV_EVENT_SMALL.raw_as_largest_type();
        assert_size(event_raw, 4);
        assert_eq!(event_raw, 0xFFFF);
    }

    #[test]
    fn as_largest_group_id() {
        let group_id = HIGH_SEV_EVENT.group_id_as_largest_type();
        assert_size(group_id, 2);
        assert_eq!(group_id, 0x3FFF);
    }

    #[test]
    fn as_largest_group_id_small_event() {
        let group_id = HIGH_SEV_EVENT_SMALL.group_id_as_largest_type();
        assert_size(group_id, 2);
        assert_eq!(group_id, 0x3F);
    }

    #[test]
    fn write_to_buf() {
        let mut buf: [u8; 4] = [0; 4];
        assert!(HIGH_SEV_EVENT.write_to_bytes(&mut buf).is_ok());
        let val_from_raw = u32::from_be_bytes(buf);
        assert_eq!(val_from_raw, 0xFFFFFFFF);
    }

    #[test]
    fn write_to_buf_small() {
        let mut buf: [u8; 2] = [0; 2];
        assert!(HIGH_SEV_EVENT_SMALL.write_to_bytes(&mut buf).is_ok());
        let val_from_raw = u16::from_be_bytes(buf);
        assert_eq!(val_from_raw, 0xFFFF);
    }

    #[test]
    fn write_to_buf_insufficient_buf() {
        let mut buf: [u8; 3] = [0; 3];
        let err = HIGH_SEV_EVENT.write_to_bytes(&mut buf);
        assert!(err.is_err());
        let err = err.unwrap_err();
        if let ByteConversionError::ToSliceTooSmall(missmatch) = err {
            assert_eq!(missmatch.expected, 4);
            assert_eq!(missmatch.found, 3);
        }
    }

    #[test]
    fn write_to_buf_small_insufficient_buf() {
        let mut buf: [u8; 1] = [0; 1];
        let err = HIGH_SEV_EVENT_SMALL.write_to_bytes(&mut buf);
        assert!(err.is_err());
        let err = err.unwrap_err();
        if let ByteConversionError::ToSliceTooSmall(missmatch) = err {
            assert_eq!(missmatch.expected, 2);
            assert_eq!(missmatch.found, 1);
        }
    }

    #[test]
    fn severity_from_invalid_raw_val() {
        let invalid = 0xFF;
        assert!(Severity::try_from(invalid).is_err());
        let invalid = Severity::HIGH as u8 + 1;
        assert!(Severity::try_from(invalid).is_err());
    }
}
