use crate::pool::{PoolProvider, StoreAddr, StoreError};
use alloc::collections::btree_map::{Entry, Range};
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;
use spacepackets::time::UnixTimestamp;
use std::collections::BTreeMap;
#[cfg(feature = "std")]
use std::time::SystemTimeError;

/// This is the core data structure for scheduling PUS telecommands with [alloc] support.
///
/// The ECSS standard specifies that the PUS scheduler can be enabled and disabled.
/// A disabled scheduler should still delete commands where the execution time has been reached
/// but should not release them to be executed.
///
/// Currently, sub-schedules and groups are not supported.
#[derive(Debug)]
pub struct PusScheduler {
    tc_map: BTreeMap<UnixTimestamp, Vec<StoreAddr>>,
    current_time: UnixTimestamp,
    time_margin: Duration,
    enabled: bool,
}

impl PusScheduler {
    /// Create a new PUS scheduler.
    ///
    /// # Arguments
    ///
    /// * `init_current_time` - The time to initialize the scheduler with.
    /// * `time_margin` - This time margin is used when inserting new telecommands into the
    ///      schedule. If the release time of a new telecommand is earlier than the time margin
    ///      added to the current time, it will not be inserted into the schedule.
    pub fn new(init_current_time: UnixTimestamp, time_margin: Duration) -> Self {
        PusScheduler {
            tc_map: Default::default(),
            current_time: init_current_time,
            time_margin,
            enabled: true,
        }
    }

    /// Like [Self::new], but sets the `init_current_time` parameter to the current system time.
    #[cfg(feature = "std")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
    pub fn new_with_current_init_time(time_margin: Duration) -> Result<Self, SystemTimeError> {
        Ok(Self::new(UnixTimestamp::from_now()?, time_margin))
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

    /// This will disable the scheduler and clear the schedule as specified in 6.11.4.4.
    /// Be careful with this command as it will delete all the commands in the schedule.
    ///
    /// The holding store for the telecommands needs to be passed so all the stored telecommands
    /// can be deleted to avoid a memory leak. If at last one deletion operation fails, the error
    /// will be returned but the method will still try to delete all the commands in the schedule.
    pub fn reset(&mut self, store: &mut impl PoolProvider) -> Result<(), StoreError> {
        self.enabled = false;
        let mut deletion_ok = Ok(());
        for tc_lists in &mut self.tc_map {
            for tc in tc_lists.1 {
                let res = store.delete(*tc);
                if res.is_err() {
                    deletion_ok = res;
                }
            }
        }
        self.tc_map.clear();
        deletion_ok
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

    /// Utility method which calls [Self::telecommands_to_release] and then calls a releaser
    /// closure for each telecommand which should be released. This function will also delete
    /// the telecommands from the holding store after calling the release closure.
    pub fn release_telecommands<R: FnMut(bool, &StoreAddr)>(
        &mut self,
        mut releaser: R,
        store: &mut impl PoolProvider,
    ) -> Result<u64, (u64, StoreError)> {
        let tcs_to_release = self.telecommands_to_release();
        let mut released_tcs = 0;
        let mut store_error = Ok(());
        for tc in tcs_to_release {
            for addr in tc.1 {
                releaser(self.enabled, addr);
                released_tcs += 1;
                let res = store.delete(*addr);
                if res.is_err() {
                    store_error = res;
                }
            }
        }
        self.tc_map.retain(|k, _| k > &self.current_time);
        store_error
            .map(|_| released_tcs)
            .map_err(|e| (released_tcs, e))
    }
}

#[cfg(test)]
mod tests {
    use crate::pool::{LocalPool, PoolCfg, PoolProvider, StoreAddr};
    use crate::pus::scheduling::PusScheduler;
    use alloc::vec::Vec;
    use spacepackets::time::UnixTimestamp;
    use std::time::Duration;
    #[allow(unused_imports)]
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
        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let first_addr = pool.add(&[0, 1, 2]).unwrap();
        let worked = scheduler.insert_tc(UnixTimestamp::new_only_seconds(100), first_addr.clone());

        assert!(worked);

        let second_addr = pool.add(&[2, 3, 4]).unwrap();
        let worked = scheduler.insert_tc(UnixTimestamp::new_only_seconds(200), second_addr.clone());

        assert!(worked);

        let third_addr = pool.add(&[5, 6, 7]).unwrap();
        let worked = scheduler.insert_tc(UnixTimestamp::new_only_seconds(300), third_addr.clone());

        assert!(worked);

        assert_eq!(scheduler.num_scheduled_telecommands(), 3);
        assert!(scheduler.is_enabled());
        scheduler.reset(&mut pool).expect("deletion of TCs failed");
        assert!(!scheduler.is_enabled());
        assert_eq!(scheduler.num_scheduled_telecommands(), 0);
        assert!(!pool.has_element_at(&first_addr).unwrap());
        assert!(!pool.has_element_at(&second_addr).unwrap());
        assert!(!pool.has_element_at(&third_addr).unwrap());
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

    fn common_check(
        enabled: bool,
        store_addr: &StoreAddr,
        expected_store_addrs: Vec<StoreAddr>,
        counter: &mut usize,
    ) {
        assert_eq!(enabled, true);
        assert!(expected_store_addrs.contains(store_addr));
        *counter += 1;
    }
    #[test]
    fn release_basic() {
        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let first_addr = pool.add(&[2, 2, 2]).unwrap();
        scheduler.insert_tc(UnixTimestamp::new_only_seconds(100), first_addr);

        let second_addr = pool.add(&[5, 6, 7]).unwrap();
        scheduler.insert_tc(UnixTimestamp::new_only_seconds(200), second_addr);

        let mut i = 0;
        let mut test_closure_1 = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![first_addr], &mut i);
        };

        // test 1: too early, no tcs
        scheduler.update_time(UnixTimestamp::new_only_seconds(99));

        scheduler
            .release_telecommands(&mut test_closure_1, &mut pool)
            .expect("deletion failed");

        // test 2: exact time stamp of tc, releases 1 tc
        scheduler.update_time(UnixTimestamp::new_only_seconds(100));

        let mut released = scheduler
            .release_telecommands(&mut test_closure_1, &mut pool)
            .expect("deletion failed");
        assert_eq!(released, 1);
        assert!(!pool.has_element_at(&first_addr).unwrap());

        // test 3, late timestamp, release 1 overdue tc
        let mut test_closure_2 = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![second_addr], &mut i);
        };

        scheduler.update_time(UnixTimestamp::new_only_seconds(206));

        released = scheduler
            .release_telecommands(&mut test_closure_2, &mut pool)
            .expect("deletion failed");
        assert_eq!(released, 1);
        assert!(!pool.has_element_at(&second_addr).unwrap());

        //test 4: no tcs left
        scheduler
            .release_telecommands(&mut test_closure_2, &mut pool)
            .expect("deletion failed");

        // check that 2 total tcs have been released
        assert_eq!(i, 2);
    }

    #[test]
    fn release_multi_with_same_time() {
        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let first_addr = pool.add(&[2, 2, 2]).unwrap();
        scheduler.insert_tc(UnixTimestamp::new_only_seconds(100), first_addr);

        let second_addr = pool.add(&[2, 2, 2]).unwrap();
        scheduler.insert_tc(UnixTimestamp::new_only_seconds(100), second_addr);

        let mut i = 0;
        let mut test_closure = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![first_addr, second_addr], &mut i);
        };

        // test 1: too early, no tcs
        scheduler.update_time(UnixTimestamp::new_only_seconds(99));

        let mut released = scheduler
            .release_telecommands(&mut test_closure, &mut pool)
            .expect("deletion failed");
        assert_eq!(released, 0);

        // test 2: exact time stamp of tc, releases 2 tc
        scheduler.update_time(UnixTimestamp::new_only_seconds(100));

        released = scheduler
            .release_telecommands(&mut test_closure, &mut pool)
            .expect("deletion failed");
        assert_eq!(released, 2);
        assert!(!pool.has_element_at(&first_addr).unwrap());
        assert!(!pool.has_element_at(&second_addr).unwrap());

        //test 3: no tcs left
        released = scheduler
            .release_telecommands(&mut test_closure, &mut pool)
            .expect("deletion failed");
        assert_eq!(released, 0);

        // check that 2 total tcs have been released
        assert_eq!(i, 2);
    }
}
