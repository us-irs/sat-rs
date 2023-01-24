use crate::pool::StoreAddr;
use alloc::collections::btree_map::{Entry, Range};
use core::time::Duration;
use spacepackets::time::UnixTimestamp;
use std::collections::BTreeMap;
use std::time::SystemTimeError;
use std::vec;
use std::vec::Vec;

#[derive(Debug)]
pub struct PusScheduler {
    tc_map: BTreeMap<UnixTimestamp, Vec<StoreAddr>>,
    current_time: UnixTimestamp,
    time_margin: Duration,
    enabled: bool,
}

impl PusScheduler {
    pub fn new(init_current_time: UnixTimestamp, time_margin: Duration) -> Self {
        PusScheduler {
            tc_map: Default::default(),
            current_time: init_current_time,
            time_margin,
            enabled: true,
        }
    }

    pub fn num_scheduled_telecommands(&self) -> u64 {
        let mut num_entries = 0;
        for entries in &self.tc_map {
            num_entries += entries.1.len() as u64;
        }
        num_entries
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

    pub fn current_time(&self) -> &UnixTimestamp {
        &self.current_time
    }

    pub fn insert_tc(&mut self, time_stamp: UnixTimestamp, addr: StoreAddr) -> bool {
        if time_stamp < self.current_time + self.time_margin {
            return false;
        }
        match self.tc_map.entry(time_stamp) {
            Entry::Vacant(e) => {
                e.insert(vec![addr]);
            }
            Entry::Occupied(mut v) => {
                v.get_mut().push(addr);
            }
        }
        true
    }

    pub fn telecommands_to_release(&self) -> Range<'_, UnixTimestamp, Vec<StoreAddr>> {
        self.tc_map.range(..=self.current_time)
    }

    #[cfg(feature = "std")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
    pub fn update_time_from_now(&mut self) -> Result<(), SystemTimeError> {
        self.current_time = UnixTimestamp::from_now()?;
        Ok(())
    }

    pub fn release_telecommands<R: FnMut(bool, &StoreAddr)>(&mut self, mut releaser: R) {
        let tcs_to_release = self.telecommands_to_release();
        for tc in tcs_to_release {
            for addr in tc.1 {
                releaser(self.enabled, addr);
            }
        }
        self.tc_map.retain(|k, _| k > &self.current_time);
    }
}

#[cfg(test)]
mod tests {
    use crate::pool::StoreAddr;
    use crate::pus::scheduling::PusScheduler;
    use spacepackets::ecss::PacketTypeCodes::UnsignedInt;
    use spacepackets::time::UnixTimestamp;
    use std::sync::mpsc;
    use std::sync::mpsc::{channel, Receiver, TryRecvError};
    use std::time::Duration;
    use std::vec::Vec;
    use std::{println, vec};

    #[test]
    fn basic() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));
        assert!(scheduler.is_enabled());
        scheduler.disable();
        assert!(!scheduler.is_enabled());
    }

    #[test]
    fn reset() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let worked = scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(100),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 1,
            },
        );

        assert!(worked);

        let worked = scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(200),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 2,
            },
        );

        assert!(worked);

        let worked = scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(300),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 2,
            },
        );

        assert!(worked);

        assert_eq!(scheduler.num_scheduled_telecommands(), 3);
        assert!(scheduler.is_enabled());
        scheduler.reset();
        assert!(!scheduler.is_enabled());
        assert_eq!(scheduler.num_scheduled_telecommands(), 0);
    }

    #[test]
    fn insert_multi_with_same_time() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let worked = scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(100),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 1,
            },
        );

        assert!(worked);

        let worked = scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(100),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 2,
            },
        );

        assert!(worked);

        let worked = scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(300),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 2,
            },
        );

        assert!(worked);

        assert_eq!(scheduler.num_scheduled_telecommands(), 3);
    }

    #[test]
    fn time() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));
        let time = UnixTimestamp::new(1, 2).unwrap();
        scheduler.update_time(time);
        assert_eq!(scheduler.current_time(), &time);
    }

    #[test]
    fn release_basic() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(100),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 1,
            },
        );

        scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(200),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 2,
            },
        );

        let mut i = 0;
        let mut test_closure_1 = |boolvar: bool, store_addr: &StoreAddr| {
            assert_eq!(boolvar, true);
            assert_eq!(
                store_addr,
                &StoreAddr {
                    pool_idx: 0,
                    packet_idx: 1,
                }
            );
            i += 1;
        };

        // test 1: too early, no tcs
        scheduler.update_time(UnixTimestamp::new_only_seconds(99));

        scheduler.release_telecommands(&mut test_closure_1);

        // test 2: exact time stamp of tc, releases 1 tc
        scheduler.update_time(UnixTimestamp::new_only_seconds(100));

        scheduler.release_telecommands(&mut test_closure_1);

        // test 3, late timestamp, release 1 overdue tc
        let mut test_closure_2 = |boolvar: bool, store_addr: &StoreAddr| {
            assert_eq!(boolvar, true);
            assert_eq!(
                store_addr,
                &StoreAddr {
                    pool_idx: 0,
                    packet_idx: 2,
                }
            );
            i += 1;
        };

        scheduler.update_time(UnixTimestamp::new_only_seconds(206));

        scheduler.release_telecommands(&mut test_closure_2);

        //test 4: no tcs left
        scheduler.release_telecommands(&mut test_closure_2);

        // check that 2 total tcs have been released
        assert_eq!(i, 2);
    }

    #[test]
    fn release_multi_with_same_time() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(100),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 1,
            },
        );

        scheduler.insert_tc(
            UnixTimestamp::new_only_seconds(100),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 1,
            },
        );

        let mut i = 0;
        let mut test_closure = |boolvar: bool, store_addr: &StoreAddr| {
            assert_eq!(boolvar, true);
            assert_eq!(
                store_addr,
                &StoreAddr {
                    pool_idx: 0,
                    packet_idx: 1,
                }
            );
            i += 1;
        };

        // test 1: too early, no tcs
        scheduler.update_time(UnixTimestamp::new_only_seconds(99));

        scheduler.release_telecommands(&mut test_closure);

        // test 2: exact time stamp of tc, releases 2 tc
        scheduler.update_time(UnixTimestamp::new_only_seconds(100));

        scheduler.release_telecommands(&mut test_closure);

        //test 3: no tcs left
        scheduler.release_telecommands(&mut test_closure);

        // check that 2 total tcs have been released
        assert_eq!(i, 2);
    }
}
