//! Event support module
use num::pow;

pub type GroupId = u16;
pub type UniqueId = u16;
pub type EventRaw = u32;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Severity {
    INFO = 1,
    LOW = 2,
    MEDIUM = 3,
    HIGH = 4,
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Event {
    severity: Severity,
    group_id: GroupId,
    unique_id: UniqueId,
}

impl Event {
    /// Generate an event. The raw representation of an event has 32 bits.
    /// If the passed group ID is invalid (too large), None wil be returned
    ///
    /// # Parameter
    ///
    /// * `severity`: Each event has a [severity][Severity]. The raw value of the severity will
    ///        be stored inside the uppermost 3 bits of the raw event ID
    /// * `group_id`: Related events can be grouped using a group ID. The group ID will occupy the
    ///        next 13 bits after the severity. Therefore, the size is limited by dec 8191 hex 0x1FFF.
    /// * `unique_id`: Each event has a unique 16 bit ID occupying the last 16 bits of the
    ///       raw event ID
    pub fn new(severity: Severity, group_id: GroupId, unique_id: UniqueId) -> Option<Event> {
        if group_id > (pow::pow(2u8 as u16, 13) - 1) {
            return None;
        }
        Some(Event {
            severity,
            group_id,
            unique_id,
        })
    }

    /// Retrieve the severity of an event. Returns None if that severity bit field of the raw event
    /// ID is invalid
    pub fn severity(&self) -> Severity {
        self.severity
    }

    pub fn group_id(&self) -> GroupId {
        self.group_id
    }

    pub fn unique_id(&self) -> UniqueId {
        self.unique_id
    }

    pub fn raw(&self) -> EventRaw {
        (((self.severity as u32) << 29) as u32
            | ((self.group_id as u32) << 16) as u32
            | self.unique_id as u32) as EventRaw
    }
}

impl TryFrom<EventRaw> for Event {
    type Error = ();

    fn try_from(raw: u32) -> Result<Self, Self::Error> {
        let severity: Option<Severity> = (((raw >> 29) & 0b111) as u8).try_into().ok();
        if severity.is_none() {
            return Err(());
        }
        let group_id = ((raw >> 16) & 0x1FFF) as u16;
        let unique_id = (raw & 0xFFFF) as u16;
        Event::new(severity.unwrap(), group_id, unique_id).ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use super::Event;
    use crate::events::Severity;

    #[test]
    fn test_events() {
        let event = Event::new(Severity::INFO, 0, 0).unwrap();
        assert_eq!(event.severity(), Severity::INFO);
        assert_eq!(event.unique_id(), 0);
        assert_eq!(event.group_id(), 0);

        let raw_event = event.raw();
        assert_eq!(raw_event, 0x20000000);
        let conv_from_raw = Event::try_from(raw_event);
        assert!(conv_from_raw.is_ok());
        let opt_event = conv_from_raw.ok();
        assert!(opt_event.is_some());
        let event = opt_event.unwrap();
        assert_eq!(event.severity(), Severity::INFO);
        assert_eq!(event.unique_id(), 0);
        assert_eq!(event.group_id(), 0);

        let event = Event::new(Severity::HIGH, 0x1FFF, 0xFFFF).unwrap();
        assert_eq!(event.severity(), Severity::HIGH);
        assert_eq!(event.group_id(), 0x1FFF);
        assert_eq!(event.unique_id(), 0xFFFF);
        let raw_event = event.raw();
        assert_eq!(raw_event, 0x9FFFFFFF);
        let conv_from_raw = Event::try_from(raw_event);
        assert!(conv_from_raw.is_ok());
        let opt_event = conv_from_raw.ok();
        assert!(opt_event.is_some());
        let event = opt_event.unwrap();
        assert_eq!(event.severity(), Severity::HIGH);
        assert_eq!(event.group_id(), 0x1FFF);
        assert_eq!(event.unique_id(), 0xFFFF);
    }
}
