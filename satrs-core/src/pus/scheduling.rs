use crate::pool::StoreAddr;
use spacepackets::time::cds;
use spacepackets::time::cds::TimeProvider;
use std::collections::BTreeMap;

pub struct PusScheduler {
    tc_map: BTreeMap<cds::TimeProvider, StoreAddr>,
}

impl PusScheduler {
    pub fn insert_tc(&mut self, time_stamp: TimeProvider, addr: StoreAddr) {
        self.tc_map.insert(time_stamp, addr);
    }
}

#[cfg(test)]
mod tests {
    use crate::pus::scheduling::PusScheduler;
    use std::collections::BTreeMap;

    #[test]
    fn wtf() {
        let scheduler = PusScheduler {
            tc_map: BTreeMap::new(),
        };
    }
}
