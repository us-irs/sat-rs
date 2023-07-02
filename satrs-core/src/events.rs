//! Event support module
//!
//! This module includes the basic event structs [EventU32] and [EventU16] and versions with the
//! ECSS severity levels as a type parameter. These structs are simple abstractions on top of the
//! [u32] and [u16] types where the raw value is the unique identifier for a particular event.
//! The abstraction also allows to group related events using a group ID, and the severity
//! of an event is encoded inside the raw value itself with four possible [Severity] levels:
//!
//!  - INFO
//!  - LOW
//!  - MEDIUM
//!  - HIGH
//!
//! All event structs implement the [EcssEnumeration] trait and can be created as constants.
//! This allows to easily create a static list of constant events which can then be used to generate
//! event telemetry using the PUS event manager modules.
//!
//! # Examples
//!
//! ```
//! use satrs_core::events::{EventU16, EventU32, EventU32TypedSev, Severity, SeverityHigh, SeverityInfo};
//!
//! const MSG_RECVD: EventU32TypedSev<SeverityInfo> = EventU32TypedSev::const_new(1, 0);
//! const MSG_FAILED: EventU32 = EventU32::const_new(Severity::LOW, 1, 1);
//!
//! const TEMPERATURE_HIGH: EventU32TypedSev<SeverityHigh> = EventU32TypedSev::const_new(2, 0);
//!
//! let small_event = EventU16::new(Severity::INFO, 3, 0);
//! ```
use core::fmt::Debug;
use core::hash::Hash;
use core::marker::PhantomData;
use delegate::delegate;
use spacepackets::ecss::EcssEnumeration;
use spacepackets::util::{ToBeBytes, UnsignedEnum};
use spacepackets::{ByteConversionError, SizeMissmatch};

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

pub trait HasSeverity: Debug + PartialEq + Eq + Copy + Clone {
    const SEVERITY: Severity;
}

/// Type level support struct
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct SeverityInfo {}
impl HasSeverity for SeverityInfo {
    const SEVERITY: Severity = Severity::INFO;
}

/// Type level support struct
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct SeverityLow {}
impl HasSeverity for SeverityLow {
    const SEVERITY: Severity = Severity::LOW;
}

/// Type level support struct
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct SeverityMedium {}
impl HasSeverity for SeverityMedium {
    const SEVERITY: Severity = Severity::MEDIUM;
}

/// Type level support struct
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct SeverityHigh {}
impl HasSeverity for SeverityHigh {
    const SEVERITY: Severity = Severity::HIGH;
}

pub trait GenericEvent: EcssEnumeration {
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
struct EventBase<RAW, GID, UID> {
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
    ) -> Result<usize, ByteConversionError> {
        if buf.len() < width {
            return Err(ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                found: buf.len(),
                expected: width,
            }));
        }
        buf.copy_from_slice(raw.to_be_bytes().as_ref());
        Ok(raw.written_len())
    }
}

impl EventBase<u32, u16, u16> {
    #[inline]
    fn raw(&self) -> u32 {
        ((self.severity as u32) << 30) | ((self.group_id as u32) << 16) | self.unique_id as u32
    }
}

impl EventBase<u16, u8, u8> {
    #[inline]
    fn raw(&self) -> u16 {
        ((self.severity as u16) << 14) | ((self.group_id as u16) << 8) | self.unique_id as u16
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

macro_rules! impl_event_provider {
    ($BaseIdent: ident, $TypedIdent: ident, $raw: ty, $gid: ty, $uid: ty) => {
        impl GenericEvent for $BaseIdent {
            type Raw = $raw;
            type GroupId = $gid;
            type UniqueId = $uid;

            event_provider_impl!();

            fn raw_as_largest_type(&self) -> LargestEventRaw {
                self.raw().into()
            }

            fn group_id_as_largest_type(&self) -> LargestGroupIdRaw {
                self.group_id().into()
            }
        }

        impl<SEVERITY: HasSeverity> GenericEvent for $TypedIdent<SEVERITY> {
            type Raw = $raw;
            type GroupId = $gid;
            type UniqueId = $uid;

            delegate!(to self.event {
                fn raw(&self) -> Self::Raw;
                fn severity(&self) -> Severity;
                fn group_id(&self) -> Self::GroupId;
                fn unique_id(&self) -> Self::UniqueId;
                fn raw_as_largest_type(&self) -> LargestEventRaw;
                fn group_id_as_largest_type(&self) -> LargestGroupIdRaw;
            });
        }
    }
}

macro_rules! try_from_impls {
    ($SevIdent: ident, $severity: path, $raw: ty, $TypedSevIdent: ident) => {
        impl TryFrom<$raw> for $TypedSevIdent<$SevIdent> {
            type Error = Severity;

            fn try_from(raw: $raw) -> Result<Self, Self::Error> {
                Self::try_from_generic($severity, raw)
            }
        }
    };
}

macro_rules! const_from_fn {
    ($from_fn_name: ident, $TypedIdent: ident, $SevIdent: ident) => {
        pub const fn $from_fn_name(event: $TypedIdent<$SevIdent>) -> Self {
            Self {
                base: event.event.base,
            }
        }
    };
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventU32 {
    base: EventBase<u32, u16, u16>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventU32TypedSev<SEVERITY> {
    event: EventU32,
    phantom: PhantomData<SEVERITY>,
}

impl<SEVERITY: HasSeverity> From<EventU32TypedSev<SEVERITY>> for EventU32 {
    fn from(e: EventU32TypedSev<SEVERITY>) -> Self {
        Self { base: e.event.base }
    }
}

impl<Severity: HasSeverity> AsRef<EventU32> for EventU32TypedSev<Severity> {
    fn as_ref(&self) -> &EventU32 {
        &self.event
    }
}

impl<Severity: HasSeverity> AsMut<EventU32> for EventU32TypedSev<Severity> {
    fn as_mut(&mut self) -> &mut EventU32 {
        &mut self.event
    }
}

impl_event_provider!(EventU32, EventU32TypedSev, u32, u16, u16);

impl EventU32 {
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
        group_id: <Self as GenericEvent>::GroupId,
        unique_id: <Self as GenericEvent>::UniqueId,
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
    pub const fn const_new(
        severity: Severity,
        group_id: <Self as GenericEvent>::GroupId,
        unique_id: <Self as GenericEvent>::UniqueId,
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

    const_from_fn!(const_from_info, EventU32TypedSev, SeverityInfo);
    const_from_fn!(const_from_low, EventU32TypedSev, SeverityLow);
    const_from_fn!(const_from_medium, EventU32TypedSev, SeverityMedium);
    const_from_fn!(const_from_high, EventU32TypedSev, SeverityHigh);
}

impl<SEVERITY: HasSeverity> EventU32TypedSev<SEVERITY> {
    /// This is similar to [EventU32::new] but the severity is a type generic, which allows to
    /// have distinct types for events with different severities
    pub fn new(
        group_id: <Self as GenericEvent>::GroupId,
        unique_id: <Self as GenericEvent>::UniqueId,
    ) -> Option<Self> {
        let event = EventU32::new(SEVERITY::SEVERITY, group_id, unique_id)?;
        Some(Self {
            event,
            phantom: PhantomData,
        })
    }

    /// Const version of [Self::new], but panics on invalid group ID input values.
    pub const fn const_new(
        group_id: <Self as GenericEvent>::GroupId,
        unique_id: <Self as GenericEvent>::UniqueId,
    ) -> Self {
        let event = EventU32::const_new(SEVERITY::SEVERITY, group_id, unique_id);
        Self {
            event,
            phantom: PhantomData,
        }
    }

    fn try_from_generic(expected: Severity, raw: u32) -> Result<Self, Severity> {
        let severity = Severity::try_from(((raw >> 30) & 0b11) as u8).unwrap();
        if severity != expected {
            return Err(severity);
        }
        Ok(Self::const_new(
            ((raw >> 16) & 0x3FFF) as u16,
            (raw & 0xFFFF) as u16,
        ))
    }
}

impl From<u32> for EventU32 {
    fn from(raw: u32) -> Self {
        // Severity conversion from u8 should never fail
        let severity = Severity::try_from(((raw >> 30) & 0b11) as u8).unwrap();
        let group_id = ((raw >> 16) & 0x3FFF) as u16;
        let unique_id = (raw & 0xFFFF) as u16;
        // Sanitized input, should never fail
        Self::const_new(severity, group_id, unique_id)
    }
}

try_from_impls!(SeverityInfo, Severity::INFO, u32, EventU32TypedSev);
try_from_impls!(SeverityLow, Severity::LOW, u32, EventU32TypedSev);
try_from_impls!(SeverityMedium, Severity::MEDIUM, u32, EventU32TypedSev);
try_from_impls!(SeverityHigh, Severity::HIGH, u32, EventU32TypedSev);

impl UnsignedEnum for EventU32 {
    fn size(&self) -> usize {
        core::mem::size_of::<u32>()
    }

    fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError> {
        self.base.write_to_bytes(self.raw(), buf, self.size())
    }
}

impl EcssEnumeration for EventU32 {
    fn pfc(&self) -> u8 {
        u32::BITS as u8
    }
}

//noinspection RsTraitImplementation
impl<SEVERITY: HasSeverity> UnsignedEnum for EventU32TypedSev<SEVERITY> {
    delegate!(to self.event {
        fn size(&self) -> usize;
        fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError>;
    });
}

//noinspection RsTraitImplementation
impl<SEVERITY: HasSeverity> EcssEnumeration for EventU32TypedSev<SEVERITY> {
    delegate!(to self.event {
        fn pfc(&self) -> u8;
    });
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventU16 {
    base: EventBase<u16, u8, u8>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventU16TypedSev<SEVERITY> {
    event: EventU16,
    phantom: PhantomData<SEVERITY>,
}

impl<Severity: HasSeverity> AsRef<EventU16> for EventU16TypedSev<Severity> {
    fn as_ref(&self) -> &EventU16 {
        &self.event
    }
}

impl<Severity: HasSeverity> AsMut<EventU16> for EventU16TypedSev<Severity> {
    fn as_mut(&mut self) -> &mut EventU16 {
        &mut self.event
    }
}

impl EventU16 {
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
        group_id: <Self as GenericEvent>::GroupId,
        unique_id: <Self as GenericEvent>::UniqueId,
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

    /// Const version of [Self::new], but panics on invalid group ID input values.
    pub const fn const_new(
        severity: Severity,
        group_id: <Self as GenericEvent>::GroupId,
        unique_id: <Self as GenericEvent>::UniqueId,
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
    const_from_fn!(const_from_info, EventU16TypedSev, SeverityInfo);
    const_from_fn!(const_from_low, EventU16TypedSev, SeverityLow);
    const_from_fn!(const_from_medium, EventU16TypedSev, SeverityMedium);
    const_from_fn!(const_from_high, EventU16TypedSev, SeverityHigh);
}

impl<SEVERITY: HasSeverity> EventU16TypedSev<SEVERITY> {
    /// This is similar to [EventU16::new] but the severity is a type generic, which allows to
    /// have distinct types for events with different severities
    pub fn new(
        group_id: <Self as GenericEvent>::GroupId,
        unique_id: <Self as GenericEvent>::UniqueId,
    ) -> Option<Self> {
        let event = EventU16::new(SEVERITY::SEVERITY, group_id, unique_id)?;
        Some(Self {
            event,
            phantom: PhantomData,
        })
    }

    /// Const version of [Self::new], but panics on invalid group ID input values.
    pub const fn const_new(
        group_id: <Self as GenericEvent>::GroupId,
        unique_id: <Self as GenericEvent>::UniqueId,
    ) -> Self {
        let event = EventU16::const_new(SEVERITY::SEVERITY, group_id, unique_id);
        Self {
            event,
            phantom: PhantomData,
        }
    }

    fn try_from_generic(expected: Severity, raw: u16) -> Result<Self, Severity> {
        let severity = Severity::try_from(((raw >> 14) & 0b11) as u8).unwrap();
        if severity != expected {
            return Err(severity);
        }
        Ok(Self::const_new(
            ((raw >> 8) & 0x3F) as u8,
            (raw & 0xFF) as u8,
        ))
    }
}

impl_event_provider!(EventU16, EventU16TypedSev, u16, u8, u8);

impl UnsignedEnum for EventU16 {
    fn size(&self) -> usize {
        core::mem::size_of::<u16>()
    }

    fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError> {
        self.base.write_to_bytes(self.raw(), buf, self.size())
    }
}
impl EcssEnumeration for EventU16 {
    #[inline]
    fn pfc(&self) -> u8 {
        u16::BITS as u8
    }
}

//noinspection RsTraitImplementation
impl<SEVERITY: HasSeverity> UnsignedEnum for EventU16TypedSev<SEVERITY> {
    delegate!(to self.event {
        fn size(&self) -> usize;
        fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError>;
    });
}

//noinspection RsTraitImplementation
impl<SEVERITY: HasSeverity> EcssEnumeration for EventU16TypedSev<SEVERITY> {
    delegate!(to self.event {
        fn pfc(&self) -> u8;
    });
}

impl From<u16> for EventU16 {
    fn from(raw: <Self as GenericEvent>::Raw) -> Self {
        let severity = Severity::try_from(((raw >> 14) & 0b11) as u8).unwrap();
        let group_id = ((raw >> 8) & 0x3F) as u8;
        let unique_id = (raw & 0xFF) as u8;
        // Sanitized input, new call should never fail
        Self::const_new(severity, group_id, unique_id)
    }
}

try_from_impls!(SeverityInfo, Severity::INFO, u16, EventU16TypedSev);
try_from_impls!(SeverityLow, Severity::LOW, u16, EventU16TypedSev);
try_from_impls!(SeverityMedium, Severity::MEDIUM, u16, EventU16TypedSev);
try_from_impls!(SeverityHigh, Severity::HIGH, u16, EventU16TypedSev);

impl<Severity: HasSeverity> PartialEq<EventU32> for EventU32TypedSev<Severity> {
    #[inline]
    fn eq(&self, other: &EventU32) -> bool {
        self.raw() == other.raw()
    }
}

impl<Severity: HasSeverity> PartialEq<EventU32TypedSev<Severity>> for EventU32 {
    #[inline]
    fn eq(&self, other: &EventU32TypedSev<Severity>) -> bool {
        self.raw() == other.raw()
    }
}

impl<Severity: HasSeverity> PartialEq<EventU16> for EventU16TypedSev<Severity> {
    #[inline]
    fn eq(&self, other: &EventU16) -> bool {
        self.raw() == other.raw()
    }
}

impl<Severity: HasSeverity> PartialEq<EventU16TypedSev<Severity>> for EventU16 {
    #[inline]
    fn eq(&self, other: &EventU16TypedSev<Severity>) -> bool {
        self.raw() == other.raw()
    }
}

#[cfg(test)]
mod tests {
    use super::EventU32TypedSev;
    use super::*;
    use spacepackets::ecss::EcssEnumeration;
    use spacepackets::ByteConversionError;
    use std::mem::size_of;

    fn assert_size<T>(_: T, val: usize) {
        assert_eq!(size_of::<T>(), val);
    }

    const INFO_EVENT: EventU32TypedSev<SeverityInfo> = EventU32TypedSev::const_new(0, 0);
    const INFO_EVENT_SMALL: EventU16TypedSev<SeverityInfo> = EventU16TypedSev::const_new(0, 0);
    const HIGH_SEV_EVENT: EventU32TypedSev<SeverityHigh> =
        EventU32TypedSev::const_new(0x3FFF, 0xFFFF);
    const HIGH_SEV_EVENT_SMALL: EventU16TypedSev<SeverityHigh> =
        EventU16TypedSev::const_new(0x3F, 0xff);

    /// This working is a test in itself.
    const INFO_REDUCED: EventU32 = EventU32::const_from_info(INFO_EVENT);

    #[test]
    fn test_normal_from_raw_conversion() {
        let conv_from_raw = EventU32TypedSev::<SeverityInfo>::try_from(INFO_EVENT.raw())
            .expect("Creating typed EventU32 failed");
        assert_eq!(conv_from_raw, INFO_EVENT);
    }

    #[test]
    fn test_small_from_raw_conversion() {
        let conv_from_raw = EventU16TypedSev::<SeverityInfo>::try_from(INFO_EVENT_SMALL.raw())
            .expect("Creating typed EventU16 failed");
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
        assert!(EventU32TypedSev::<SeverityMedium>::new(2_u16.pow(14), 0).is_none());
    }

    #[test]
    fn invalid_group_id_small() {
        assert!(EventU16TypedSev::<SeverityMedium>::new(2_u8.pow(6), 0).is_none());
    }

    #[test]
    fn regular_new() {
        assert_eq!(
            EventU32TypedSev::<SeverityInfo>::new(0, 0).expect("Creating regular event failed"),
            INFO_EVENT
        );
    }

    #[test]
    fn small_new() {
        assert_eq!(
            EventU16TypedSev::<SeverityInfo>::new(0, 0).expect("Creating regular event failed"),
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
        assert!(HIGH_SEV_EVENT.write_to_be_bytes(&mut buf).is_ok());
        let val_from_raw = u32::from_be_bytes(buf);
        assert_eq!(val_from_raw, 0xFFFFFFFF);
    }

    #[test]
    fn write_to_buf_small() {
        let mut buf: [u8; 2] = [0; 2];
        assert!(HIGH_SEV_EVENT_SMALL.write_to_be_bytes(&mut buf).is_ok());
        let val_from_raw = u16::from_be_bytes(buf);
        assert_eq!(val_from_raw, 0xFFFF);
    }

    #[test]
    fn write_to_buf_insufficient_buf() {
        let mut buf: [u8; 3] = [0; 3];
        let err = HIGH_SEV_EVENT.write_to_be_bytes(&mut buf);
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
        let err = HIGH_SEV_EVENT_SMALL.write_to_be_bytes(&mut buf);
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

    #[test]
    fn reduction() {
        let event = EventU32TypedSev::<SeverityInfo>::const_new(1, 1);
        let raw = event.raw();
        let reduced: EventU32 = event.into();
        assert_eq!(reduced.group_id(), 1);
        assert_eq!(reduced.unique_id(), 1);
        assert_eq!(raw, reduced.raw());
    }

    #[test]
    fn const_reducation() {
        assert_eq!(INFO_REDUCED.raw(), INFO_EVENT.raw());
    }
}
