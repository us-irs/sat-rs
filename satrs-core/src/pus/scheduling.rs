use crate::pool::StoreAddr;
use spacepackets::time::UnixTimestamp;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct PusScheduler {
    tc_map: BTreeMap<UnixTimestamp, StoreAddr>,
    current_time: UnixTimestamp,
    enabled: bool,
}

impl PusScheduler {
    pub fn new(init_current_time: UnixTimestamp) -> Self {
        PusScheduler {
            tc_map: Default::default(),
            current_time: init_current_time,
            enabled: true,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn reset(&mut self) {
        self.enabled = false;
        self.tc_map.clear();
    }

    pub fn update_time(&mut self, current_time: UnixTimestamp) {
        self.current_time = current_time;
    }

    pub fn insert_tc(&mut self, time_stamp: UnixTimestamp, addr: StoreAddr) {
        self.tc_map.insert(time_stamp, addr);
    }
}

#[cfg(test)]
mod tests {
    use crate::pus::scheduling::PusScheduler;
    use spacepackets::time::UnixTimestamp;

    #[test]
    fn basic() {
        let mut scheduler = PusScheduler::new(UnixTimestamp::new_only_seconds(0));
        assert!(scheduler.is_enabled());
        scheduler.disable();
        assert!(!scheduler.is_enabled());
    }
}
