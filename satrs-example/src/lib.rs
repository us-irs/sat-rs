use satrs::spacepackets::time::{cds::CdsTime, TimeWriter};

pub mod config;
pub mod ids;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum DeviceMode {
    Off = 0,
    On = 1,
    Normal = 2,
}

pub struct TimestampHelper {
    stamper: CdsTime,
    time_stamp: [u8; 7],
}

impl TimestampHelper {
    pub fn stamp(&self) -> &[u8] {
        &self.time_stamp
    }

    pub fn update_from_now(&mut self) {
        self.stamper
            .update_from_now()
            .expect("Updating timestamp failed");
        self.stamper
            .write_to_bytes(&mut self.time_stamp)
            .expect("Writing timestamp failed");
    }
}

impl Default for TimestampHelper {
    fn default() -> Self {
        Self {
            stamper: CdsTime::now_with_u16_days().expect("creating time stamper failed"),
            time_stamp: Default::default(),
        }
    }
}
