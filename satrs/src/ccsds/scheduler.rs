use core::{hash::Hash, time::Duration};

#[cfg(feature = "alloc")]
pub use alloc_mod::*;
use spacepackets::{
    ByteConversionError, PacketId, PacketSequenceControl,
    time::{TimestampError, UnixTime},
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ScheduleError {
    /// The release time is within the time-margin added on top of the current time.
    /// The first parameter is the current time, the second one the time margin, and the third one
    /// the release time.
    #[error("release time in margin")]
    ReleaseTimeInTimeMargin {
        current_time: UnixTime,
        time_margin: Duration,
        release_time: UnixTime,
    },
    /// Nested time-tagged commands are not allowed.
    #[error("nested scheduled tc")]
    NestedScheduledTc,
    #[error("tc data empty")]
    TcDataEmpty,
    #[error("timestamp error: {0}")]
    TimestampError(#[from] TimestampError),
    #[error("wrong subservice number {0}")]
    WrongSubservice(u8),
    #[error("wrong service number {0}")]
    WrongService(u8),
    #[error("byte conversion error: {0}")]
    ByteConversionError(#[from] ByteConversionError),
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CcsdsPacketId {
    pub packet_id: PacketId,
    pub psc: PacketSequenceControl,
    pub crc16: u16,
}

impl Hash for CcsdsPacketId {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.packet_id.hash(state);
        self.psc.raw().hash(state);
        self.crc16.hash(state);
    }
}

pub mod alloc_mod {
    use core::time::Duration;
    #[cfg(feature = "std")]
    use std::time::SystemTimeError;

    use spacepackets::time::UnixTime;

    use crate::ccsds::scheduler::CcsdsPacketId;

    pub struct CcsdsScheduler {
        tc_map: alloc::collections::BTreeMap<
            UnixTime,
            alloc::vec::Vec<(CcsdsPacketId, alloc::vec::Vec<u8>)>,
        >,
        packet_limit: usize,
        pub(crate) current_time: UnixTime,
        time_margin: Duration,
        enabled: bool,
    }

    impl CcsdsScheduler {
        pub fn new(current_time: UnixTime, packet_limit: usize, time_margin: Duration) -> Self {
            Self {
                tc_map: alloc::collections::BTreeMap::new(),
                packet_limit,
                current_time,
                time_margin,
                enabled: true,
            }
        }

        /// Like [Self::new], but sets the `init_current_time` parameter to the current system time.
        #[cfg(feature = "std")]
        pub fn new_with_current_init_time(
            packet_limit: usize,
            time_margin: Duration,
        ) -> Result<Self, SystemTimeError> {
            Ok(Self::new(UnixTime::now()?, packet_limit, time_margin))
        }

        pub fn num_of_entries(&self) -> usize {
            self.tc_map
                .values()
                .map(|v| v.iter().map(|(_, v)| v.len()).sum::<usize>())
                .sum()
        }

        #[inline]
        pub fn enable(&mut self) {
            self.enabled = true;
        }

        #[inline]
        pub fn disable(&mut self) {
            self.enabled = false;
        }

        #[inline]
        pub fn update_time(&mut self, current_time: UnixTime) {
            self.current_time = current_time;
        }

        #[inline]
        pub fn current_time(&self) -> &UnixTime {
            &self.current_time
        }

        // TODO: Implementation
        pub fn insert_telecommand(
            &mut self,
            packet_id: CcsdsPacketId,
            packet: alloc::vec::Vec<u8>,
            release_time: UnixTime,
        ) {
        }
    }
}
