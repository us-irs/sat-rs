use crate::pool::{PoolProvider, StoreAddr, StoreError};
use alloc::collections::btree_map::{Entry, Range};
use core::time::Duration;
use spacepackets::ecss::PusPacket;
use spacepackets::tc::{GenericPusTcSecondaryHeader, PusTc};
use spacepackets::time::cds::DaysLen24Bits;
use spacepackets::time::{CcsdsTimeProvider, TimeReader, TimestampError, UnixTimestamp};
use std::collections::BTreeMap;
use std::time::SystemTimeError;
use std::vec;
use std::vec::Vec;


//TODO: Move to spacepackets
#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ScheduleSubservice {
    EnableScheduling = 1,
    DisableScheduling = 2,
    ResetScheduling = 3,
    InsertActivity = 4,
    DeleteActivity = 5,

}

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
    pub fn reset(&mut self, store: &mut (impl PoolProvider + ?Sized)) -> Result<(), StoreError> {
        self.enabled = false;
        self.tc_map.clear();
        return Ok(())
    }

    pub fn update_time(&mut self, current_time: UnixTimestamp) {
        self.current_time = current_time;
    }

    pub fn current_time(&self) -> &UnixTimestamp {
        &self.current_time
    }

    pub fn insert_unwrapped_and_stored_tc(
        &mut self,
        time_stamp: UnixTimestamp,
        addr: StoreAddr,
    ) -> bool {
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

    pub fn insert_unwrapped_tc(
        &mut self,
        time_stamp: UnixTimestamp,
        tc: &[u8],
        pool: &mut (impl PoolProvider + ?Sized),
    ) -> Result<StoreAddr, ()> {
        let check_tc = PusTc::from_bytes(tc).unwrap();
        if PusPacket::service(&check_tc.0) == 11 && PusPacket::subservice(&check_tc.0) == 4 {
            // TODO: should not be able to schedule a scheduled tc
            return Err(());
        }

        match pool.add(tc) {
            Ok(addr) => {
                let worked = self.insert_unwrapped_and_stored_tc(time_stamp, addr);
                if worked {
                    return Ok(addr);
                } else {
                    return Err(());
                }
            }
            Err(err) => {
                return Err(());
            }
        }
    }

    // insert_wrapped_tc<cds::TimeProvider>()
    // <T: FnMut(&[u8]) -> (&dyn CcsdsTimeProvider)>
    pub fn insert_wrapped_tc<TimeStamp: CcsdsTimeProvider + TimeReader>(
        &mut self,
        pus_tc: &PusTc,
        pool: &mut (impl PoolProvider + ?Sized),
    ) -> Result<StoreAddr, ()> {
        if PusPacket::service(pus_tc) != 11 || PusPacket::subservice(pus_tc) != 4 {
            return Err(());
        }

        return if let Some(user_data) = pus_tc.user_data() {
            let mut stamp: TimeStamp = match TimeReader::from_bytes(user_data) {
                Ok(stamp) => stamp,
                Err(error) => return Err(()),
            };
            let unix_stamp = stamp.unix_stamp();
            let stamp_len = stamp.len_as_bytes();
            self.insert_unwrapped_tc(unix_stamp, &user_data[stamp_len..], pool)
        } else {
            Err(())
        }
    }

    pub fn insert_wrapped_tc_cds_short(
        &mut self,
        pus_tc: &PusTc,
        pool: &mut (impl PoolProvider + ?Sized),
    ) -> Result<StoreAddr, ()> {
        self.insert_wrapped_tc::<spacepackets::time::cds::TimeProvider>(pus_tc, pool)
    }

    pub fn insert_wrapped_tc_cds_long(
        &mut self,
        pus_tc: &PusTc,
        pool: &mut (impl PoolProvider + ?Sized),
    ) -> Result<StoreAddr, ()> {
        self.insert_wrapped_tc::<spacepackets::time::cds::TimeProvider<DaysLen24Bits>>(pus_tc, pool)
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
    ///
    /// # Arguments
    ///
    /// * `releaser` - Closure where the first argument is whether the scheduler is enabled and
    ///     the second argument is the store address. This closure should return whether the
    ///     command should be deleted.
    /// * `store` - The holding store of the telecommands.
    pub fn release_telecommands<R: FnMut(bool, &StoreAddr) -> bool>(
        &mut self,
        mut releaser: R,
        tc_store: &mut (impl PoolProvider + ?Sized),
    ) -> Result<u64, (u64, StoreError)> {
        let tcs_to_release = self.telecommands_to_release();
        let mut released_tcs = 0;
        let mut store_error = Ok(());
        for tc in tcs_to_release {
            for addr in tc.1 {
                let should_delete = releaser(self.enabled, addr);
                released_tcs += 1;
                if should_delete {
                    let res = tc_store.delete(*addr);
                    if res.is_err() {
                        store_error = res;
                    }
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
    use crate::pool::{LocalPool, PoolCfg, StoreAddr};
    use crate::pus::scheduling::PusScheduler;
    use spacepackets::ecss::PacketTypeCodes::UnsignedInt;
    use spacepackets::time::UnixTimestamp;
    use std::sync::mpsc;
    use std::sync::mpsc::{channel, Receiver, TryRecvError};
    use std::time::Duration;
    use std::vec::Vec;
    use std::{println, vec};
    use heapless::pool::Pool;
    use spacepackets::SpHeader;
    use spacepackets::tc::PusTc;

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
        let first_addr = pool.add(&[0, 1, 2]).unwrap();
        let worked = scheduler.insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(100), first_addr.clone());

        assert!(worked);

        let second_addr = pool.add(&[2, 3, 4]).unwrap();
        let worked = scheduler.insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(200), second_addr.clone());

        assert!(worked);

        let third_addr = pool.add(&[5, 6, 7]).unwrap();
        let worked = scheduler.insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(300), third_addr.clone());

        assert!(worked);

        assert_eq!(scheduler.num_scheduled_telecommands(), 3);
        assert!(scheduler.is_enabled());
        scheduler.reset().unwrap();
        assert!(!scheduler.is_enabled());
        assert_eq!(scheduler.num_scheduled_telecommands(), 0);
    }

    #[test]
    fn insert_multi_with_same_time() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let worked = scheduler.insert_unwrapped_and_stored_tc(
            UnixTimestamp::new_only_seconds(100),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 1,
            },
        );

        assert!(worked);

        let worked = scheduler.insert_unwrapped_and_stored_tc(
            UnixTimestamp::new_only_seconds(100),
            StoreAddr {
                pool_idx: 0,
                packet_idx: 2,
            },
        );

        assert!(worked);

        let worked = scheduler.insert_unwrapped_and_stored_tc(
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

        let first_addr = pool.add(&[2, 2, 2]).unwrap();
        scheduler.insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(100), first_addr);

        let second_addr = pool.add(&[5, 6, 7]).unwrap();
        scheduler.insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(200), second_addr);

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

        let first_addr = pool.add(&[2, 2, 2]).unwrap();
        scheduler.insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(100), first_addr);

        let second_addr = pool.add(&[2, 2, 2]).unwrap();
        scheduler.insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(100), second_addr);

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

    #[test]
    fn insert_unwrapped_tc() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));

        let addr = scheduler.insert_unwrapped_tc(UnixTimestamp::new_only_seconds(100), &[1,2,3,4], &mut pool).unwrap();

        assert_eq!(scheduler.num_scheduled_telecommands(), 1);

        scheduler.update_time(UnixTimestamp::new_only_seconds(101));

        let mut test_closure = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![addr], &mut i);
            true
        };

        scheduler.release_telecommands(&mut test_closure, &mut pool).unwrap();
    }

    fn scheduled_tc() -> PusTc<'static> {
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, 0).unwrap();
        PusTc::new_simple(&mut sph, 11, 4, None, true)
    }

    #[test]
    fn insert_wrapped_tc() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));

        let tc = base_ping_tc_simple_ctor();

        let addr = scheduler.insert_wrapped_tc( &tc, &mut pool).unwrap();

        assert_eq!(scheduler.num_scheduled_telecommands(), 1);

        scheduler.update_time(UnixTimestamp::new_only_seconds(101));

        let mut test_closure = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![addr], &mut i);
            true
        };

        scheduler.release_telecommands(&mut test_closure, &mut pool).unwrap();
    }

    fn base_ping_tc_simple_ctor() -> PusTc<'static> {
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, 0).unwrap();
        PusTc::new_simple(&mut sph, 17, 1, None, true)
    }

    fn insert_wrong_subservice() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));

        let tc = base_ping_tc_simple_ctor();

        let addr = scheduler.insert_wrapped_tc( &tc, &mut pool).unwrap();

        assert_eq!(scheduler.num_scheduled_telecommands(), 0);
    }
}
