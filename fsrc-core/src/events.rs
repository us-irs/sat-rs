//! Event support module

use spacepackets::ecss::EcssEnumeration;
use spacepackets::{ByteConversionError, SizeMissmatch};

pub type EventRaw = u32;
pub type SmallEventRaw = u16;

pub type GroupId = u16;
pub type UniqueId = u16;

pub type GroupIdSmall = u8;
pub type UniqueIdSmall = u8;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum Severity {
    INFO = 0,
    LOW = 1,
    MEDIUM = 2,
    HIGH = 3,
}

pub trait EventProvider: PartialEq + Eq + Copy + Clone {
    type Raw;
    type GroupId;
    type UniqueId;

    fn raw(&self) -> Self::Raw;
    fn severity(&self) -> Severity;
    fn group_id(&self) -> Self::GroupId;
    fn unique_id(&self) -> Self::UniqueId;

    fn raw_as_u32(&self) -> u32;
    fn group_id_as_u16(&self) -> u16;
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
pub struct Event {
    severity: Severity,
    group_id: u16,
    unique_id: u16,
}

impl EventProvider for Event {
    type Raw = u32;
    type GroupId = u16;
    type UniqueId = u16;

    fn raw(&self) -> Self::Raw {
        (((self.severity as Self::Raw) << 30) as Self::Raw
            | ((self.group_id as Self::Raw) << 16) as Self::Raw
            | self.unique_id as u32) as Self::Raw
    }

    /// Retrieve the severity of an event. Returns None if that severity bit field of the raw event
    /// ID is invalid
    fn severity(&self) -> Severity {
        self.severity
    }

    fn group_id(&self) -> Self::GroupId {
        self.group_id
    }

    fn unique_id(&self) -> Self::UniqueId {
        self.unique_id
    }

    fn raw_as_u32(&self) -> u32 {
        self.raw()
    }

    fn group_id_as_u16(&self) -> u16 {
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
            severity,
            group_id,
            unique_id,
        })
    }
}

impl TryFrom<u32> for Event {
    type Error = ();

    fn try_from(raw: u32) -> Result<Self, Self::Error> {
        let severity: Option<Severity> = (((raw >> 30) & 0b11) as u8).try_into().ok();
        if severity.is_none() {
            return Err(());
        }
        let group_id = ((raw >> 16) & 0x3FFF) as u16;
        let unique_id = (raw & 0xFFFF) as u16;
        Event::new(severity.unwrap(), group_id, unique_id).ok_or(())
    }
}

impl EcssEnumeration for Event {
    fn pfc(&self) -> u8 {
        32
    }

    fn write_to_bytes(&self, buf: &mut [u8]) -> Result<(), ByteConversionError> {
        if buf.len() < self.byte_width() {
            return Err(ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                found: buf.len(),
                expected: self.byte_width(),
            }));
        }
        buf.copy_from_slice(self.raw().to_be_bytes().as_slice());
        Ok(())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventSmall {
    severity: Severity,
    group_id: u8,
    unique_id: u8,
}

impl EventProvider for EventSmall {
    type Raw = u16;
    type GroupId = u8;
    type UniqueId = u8;

    fn raw(&self) -> Self::Raw {
        (((self.severity as Self::Raw) << 14) as Self::Raw
            | ((self.group_id as Self::Raw) << 8) as Self::Raw
            | self.unique_id as Self::Raw) as Self::Raw
    }

    /// Retrieve the severity of an event. Returns None if that severity bit field of the raw event
    /// ID is invalid
    fn severity(&self) -> Severity {
        self.severity
    }

    fn group_id(&self) -> Self::GroupId {
        self.group_id.into()
    }

    fn unique_id(&self) -> Self::UniqueId {
        self.unique_id.into()
    }

    fn raw_as_u32(&self) -> u32 {
        self.raw().into()
    }

    fn group_id_as_u16(&self) -> u16 {
        self.group_id().into()
    }
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
            severity,
            group_id,
            unique_id,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::Event;
    use crate::events::{EventProvider, Severity};

    #[test]
    fn test_events() {
        let event = Event::new(Severity::INFO, 0, 0).unwrap();
        assert_eq!(event.severity(), Severity::INFO);
        assert_eq!(event.unique_id(), 0);
        assert_eq!(event.group_id(), 0);

        let raw_event = event.raw();
        assert_eq!(raw_event, 0x00000000);
        let conv_from_raw = Event::try_from(raw_event);
        assert!(conv_from_raw.is_ok());
        let opt_event = conv_from_raw.ok();
        assert!(opt_event.is_some());
        let event = opt_event.unwrap();
        assert_eq!(event.severity(), Severity::INFO);
        assert_eq!(event.unique_id(), 0);
        assert_eq!(event.group_id(), 0);

        let event = Event::new(Severity::HIGH, 0x3FFF, 0xFFFF).unwrap();
        assert_eq!(event.severity(), Severity::HIGH);
        assert_eq!(event.group_id(), 0x3FFF);
        assert_eq!(event.unique_id(), 0xFFFF);
        let raw_event = event.raw();
        assert_eq!(raw_event, 0xFFFFFFFF);
        let conv_from_raw = Event::try_from(raw_event);
        assert!(conv_from_raw.is_ok());
        let opt_event = conv_from_raw.ok();
        assert!(opt_event.is_some());
        let event = opt_event.unwrap();
        assert_eq!(event.severity(), Severity::HIGH);
        assert_eq!(event.group_id(), 0x3FFF);
        assert_eq!(event.unique_id(), 0xFFFF);
    }
}
