//! # PUS Service 11 Scheduling Module
use crate::pool::{PoolProvider, StoreAddr, StoreError};
use alloc::collections::btree_map::{Entry, Range};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::{Debug, Display, Formatter};
use core::time::Duration;
use spacepackets::ecss::{PusError, PusPacket};
use spacepackets::tc::PusTc;
use spacepackets::time::cds::DaysLen24Bits;
use spacepackets::time::{CcsdsTimeProvider, TimeReader, TimestampError, UnixTimestamp};
use std::collections::BTreeMap;
#[cfg(feature = "std")]
use std::error::Error;
#[cfg(feature = "std")]
use std::time::SystemTimeError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    PusError(PusError),
    TimeMarginTooShort(UnixTimestamp, UnixTimestamp),
    NestedScheduledTc,
    StoreError(StoreError),
    TcDataEmpty,
    TimestampError(TimestampError),
    WrongSubservice,
    WrongService,
}

impl Display for ScheduleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            ScheduleError::PusError(e) => {
                write!(f, "Pus Error: {}", e)
            }
            ScheduleError::TimeMarginTooShort(current_time, timestamp) => {
                write!(
                    f,
                    "Error: time margin too short, current time: {:?}, time stamp: {:?}",
                    current_time, timestamp
                )
            }
            ScheduleError::NestedScheduledTc => {
                write!(f, "Error: nested scheduling is not allowed")
            }
            ScheduleError::StoreError(e) => {
                write!(f, "Store Error: {}", e)
            }
            ScheduleError::TcDataEmpty => {
                write!(f, "Error: empty Tc Data field")
            }
            ScheduleError::TimestampError(e) => {
                write!(f, "Timestamp Error: {}", e)
            }
            ScheduleError::WrongService => {
                write!(f, "Error: Service not 11.")
            }
            ScheduleError::WrongSubservice => {
                write!(f, "Error: Subservice not 4.")
            }
        }
    }
}

impl From<PusError> for ScheduleError {
    fn from(e: PusError) -> Self {
        ScheduleError::PusError(e)
    }
}

impl From<StoreError> for ScheduleError {
    fn from(e: StoreError) -> Self {
        ScheduleError::StoreError(e)
    }
}

impl From<TimestampError> for ScheduleError {
    fn from(e: TimestampError) -> Self {
        ScheduleError::TimestampError(e)
    }
}

#[cfg(feature = "std")]
impl Error for ScheduleError {}

/// This is the core data structure for scheduling PUS telecommands with [alloc] support.
///
/// It is assumed that the actual telecommand data is stored in a separate TC pool offering
/// a [crate::pool::PoolProvider] API. This data structure just tracks the store addresses and their
/// release times and offers a convenient API to insert and release telecommands and perform
/// other functionality specified by the ECSS standard in section 6.11. The time is tracked
/// as a [spacepackets::time::UnixTimestamp] but the only requirement to the timekeeping of
/// the user is that it is convertible to that timestamp.
///
/// The standard also specifies that the PUS scheduler can be enabled and disabled.
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

    pub fn reset(&mut self, store: &mut (impl PoolProvider + ?Sized)) -> Result<(), StoreError> {
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

    pub fn insert_unwrapped_and_stored_tc(
        &mut self,
        time_stamp: UnixTimestamp,
        addr: StoreAddr,
    ) -> Result<(), ScheduleError> {
        if time_stamp < self.current_time + self.time_margin {
            return Err(ScheduleError::TimeMarginTooShort(
                self.current_time,
                time_stamp,
            ));
        }
        match self.tc_map.entry(time_stamp) {
            Entry::Vacant(e) => {
                e.insert(vec![addr]);
            }
            Entry::Occupied(mut v) => {
                v.get_mut().push(addr);
            }
        }
        Ok(())
    }

    pub fn insert_unwrapped_tc(
        &mut self,
        time_stamp: UnixTimestamp,
        tc: &[u8],
        pool: &mut (impl PoolProvider + ?Sized),
    ) -> Result<StoreAddr, ScheduleError> {
        let check_tc = PusTc::from_bytes(tc)?;
        if PusPacket::service(&check_tc.0) == 11 && PusPacket::subservice(&check_tc.0) == 4 {
            return Err(ScheduleError::NestedScheduledTc);
        }

        match pool.add(tc) {
            Ok(addr) => {
                self.insert_unwrapped_and_stored_tc(time_stamp, addr)?;
                Ok(addr)
            }
            Err(err) => Err(err.into()),
        }
    }

    // <T: FnMut(&[u8]) -> (&dyn CcsdsTimeProvider)>
    pub fn insert_wrapped_tc<TimeStamp: CcsdsTimeProvider + TimeReader>(
        &mut self,
        pus_tc: &PusTc,
        pool: &mut (impl PoolProvider + ?Sized),
    ) -> Result<StoreAddr, ScheduleError> {
        if PusPacket::service(pus_tc) != 11 {
            return Err(ScheduleError::WrongService);
        }
        if PusPacket::subservice(pus_tc) != 4 {
            return Err(ScheduleError::WrongSubservice);
        }
        return if let Some(user_data) = pus_tc.user_data() {
            let stamp: TimeStamp = TimeReader::from_bytes(user_data)?;
            let unix_stamp = stamp.unix_stamp();
            let stamp_len = stamp.len_as_bytes();
            self.insert_unwrapped_tc(unix_stamp, &user_data[stamp_len..], pool)
        } else {
            Err(ScheduleError::TcDataEmpty)
        };
    }

    pub fn insert_wrapped_tc_cds_short(
        &mut self,
        pus_tc: &PusTc,
        pool: &mut (impl PoolProvider + ?Sized),
    ) -> Result<StoreAddr, ScheduleError> {
        self.insert_wrapped_tc::<spacepackets::time::cds::TimeProvider>(pus_tc, pool)
    }

    pub fn insert_wrapped_tc_cds_long(
        &mut self,
        pus_tc: &PusTc,
        pool: &mut (impl PoolProvider + ?Sized),
    ) -> Result<StoreAddr, ScheduleError> {
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
    /// the telecommands from the holding store after calling the release closure, if the scheduler
    /// is disabled.
    ///
    /// # Arguments
    ///
    /// * `releaser` - Closure where the first argument is whether the scheduler is enabled and
    ///     the second argument is the store address. This closure should return whether the
    ///     command should be deleted if the scheduler is disabled to prevent memory leaks.
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
                if should_delete && !self.is_enabled() {
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
    use crate::pool::{LocalPool, PoolCfg, PoolProvider, StoreAddr};
    use crate::pus::scheduling::{PusScheduler, ScheduleError};
    use spacepackets::tc::PusTc;
    use spacepackets::time::{cds, TimeWriter, UnixTimestamp};
    use spacepackets::SpHeader;
    use std::time::Duration;
    use std::vec::Vec;
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

        scheduler
            .insert_unwrapped_and_stored_tc(
                UnixTimestamp::new_only_seconds(100),
                first_addr.clone(),
            )
            .unwrap();

        let second_addr = pool.add(&[2, 3, 4]).unwrap();
        scheduler
            .insert_unwrapped_and_stored_tc(
                UnixTimestamp::new_only_seconds(200),
                second_addr.clone(),
            )
            .unwrap();

        let third_addr = pool.add(&[5, 6, 7]).unwrap();
        scheduler
            .insert_unwrapped_and_stored_tc(
                UnixTimestamp::new_only_seconds(300),
                third_addr.clone(),
            )
            .unwrap();

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

        scheduler
            .insert_unwrapped_and_stored_tc(
                UnixTimestamp::new_only_seconds(100),
                StoreAddr {
                    pool_idx: 0,
                    packet_idx: 1,
                },
            )
            .unwrap();

        scheduler
            .insert_unwrapped_and_stored_tc(
                UnixTimestamp::new_only_seconds(100),
                StoreAddr {
                    pool_idx: 0,
                    packet_idx: 2,
                },
            )
            .unwrap();

        scheduler
            .insert_unwrapped_and_stored_tc(
                UnixTimestamp::new_only_seconds(300),
                StoreAddr {
                    pool_idx: 0,
                    packet_idx: 2,
                },
            )
            .unwrap();

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
    fn common_check_disabled(
        enabled: bool,
        store_addr: &StoreAddr,
        expected_store_addrs: Vec<StoreAddr>,
        counter: &mut usize,
    ) {
        assert_eq!(enabled, false);
        assert!(expected_store_addrs.contains(store_addr));
        *counter += 1;
    }

    #[test]
    fn release_basic() {
        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let first_addr = pool.add(&[2, 2, 2]).unwrap();

        scheduler
            .insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(100), first_addr)
            .expect("insertion failed");

        let second_addr = pool.add(&[5, 6, 7]).unwrap();
        scheduler
            .insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(200), second_addr)
            .expect("insertion failed");

        let mut i = 0;
        let mut test_closure_1 = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![first_addr], &mut i);
            true
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
        assert!(pool.has_element_at(&first_addr).unwrap());

        // test 3, late timestamp, release 1 overdue tc
        let mut test_closure_2 = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![second_addr], &mut i);
            true
        };

        scheduler.update_time(UnixTimestamp::new_only_seconds(206));

        released = scheduler
            .release_telecommands(&mut test_closure_2, &mut pool)
            .expect("deletion failed");
        assert_eq!(released, 1);
        assert!(pool.has_element_at(&second_addr).unwrap());

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

        scheduler
            .insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(100), first_addr)
            .expect("insertion failed");

        let second_addr = pool.add(&[2, 2, 2]).unwrap();
        scheduler
            .insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(100), second_addr)
            .expect("insertion failed");

        let mut i = 0;
        let mut test_closure = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![first_addr, second_addr], &mut i);
            true
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
        assert!(pool.has_element_at(&first_addr).unwrap());
        assert!(pool.has_element_at(&second_addr).unwrap());

        //test 3: no tcs left
        released = scheduler
            .release_telecommands(&mut test_closure, &mut pool)
            .expect("deletion failed");
        assert_eq!(released, 0);

        // check that 2 total tcs have been released
        assert_eq!(i, 2);
    }

    #[test]
    fn release_with_scheduler_disabled() {
        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        scheduler.disable();

        let first_addr = pool.add(&[2, 2, 2]).unwrap();

        scheduler
            .insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(100), first_addr)
            .expect("insertion failed");

        let second_addr = pool.add(&[5, 6, 7]).unwrap();
        scheduler
            .insert_unwrapped_and_stored_tc(UnixTimestamp::new_only_seconds(200), second_addr)
            .expect("insertion failed");

        let mut i = 0;
        let mut test_closure_1 = |boolvar: bool, store_addr: &StoreAddr| {
            common_check_disabled(boolvar, store_addr, vec![first_addr], &mut i);
            true
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
            common_check_disabled(boolvar, store_addr, vec![second_addr], &mut i);
            true
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

    fn scheduled_tc(timestamp: UnixTimestamp, buf: &mut [u8]) -> PusTc {
        let cds_time = cds::TimeProvider::from_unix_secs_with_u16_days(&timestamp).unwrap();

        let len_time_stamp = cds_time.write_to_bytes(buf).unwrap();

        let len_packet = base_ping_tc_simple_ctor()
            .write_to_bytes(&mut buf[len_time_stamp..])
            .unwrap();
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, len_packet as u16).unwrap();

        PusTc::new_simple(
            &mut sph,
            11,
            4,
            Some(&buf[..len_packet + len_time_stamp]),
            true,
        )
    }

    fn wrong_tc_service(timestamp: UnixTimestamp, buf: &mut [u8]) -> PusTc {
        let cds_time = cds::TimeProvider::from_unix_secs_with_u16_days(&timestamp).unwrap();

        let len_time_stamp = cds_time.write_to_bytes(buf).unwrap();

        let len_packet = base_ping_tc_simple_ctor()
            .write_to_bytes(&mut buf[len_time_stamp..])
            .unwrap();
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, len_packet as u16).unwrap();

        PusTc::new_simple(
            &mut sph,
            12,
            4,
            Some(&buf[..len_packet + len_time_stamp]),
            true,
        )
    }

    fn wrong_tc_subservice(timestamp: UnixTimestamp, buf: &mut [u8]) -> PusTc {
        let cds_time = cds::TimeProvider::from_unix_secs_with_u16_days(&timestamp).unwrap();

        let len_time_stamp = cds_time.write_to_bytes(buf).unwrap();

        let len_packet = base_ping_tc_simple_ctor()
            .write_to_bytes(&mut buf[len_time_stamp..])
            .unwrap();
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, len_packet as u16).unwrap();

        PusTc::new_simple(
            &mut sph,
            11,
            5,
            Some(&buf[..len_packet + len_time_stamp]),
            true,
        )
    }

    fn base_ping_tc_simple_ctor() -> PusTc<'static> {
        let mut sph = SpHeader::tc_unseg(0x02, 0x34, 0).unwrap();
        PusTc::new_simple(&mut sph, 17, 1, None, true)
    }

    #[test]
    fn insert_unwrapped_tc() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));
        let mut buf: [u8; 32] = [0; 32];
        let len = base_ping_tc_simple_ctor().write_to_bytes(&mut buf).unwrap();

        let addr = scheduler
            .insert_unwrapped_tc(UnixTimestamp::new_only_seconds(100), &buf[..len], &mut pool)
            .unwrap();

        assert!(pool.has_element_at(&addr).unwrap());

        let data = pool.read(&addr).unwrap();
        let check_tc = PusTc::from_bytes(&data).expect("incorrect Pus tc raw data");
        assert_eq!(check_tc.0, base_ping_tc_simple_ctor());

        assert_eq!(scheduler.num_scheduled_telecommands(), 1);

        scheduler.update_time(UnixTimestamp::new_only_seconds(101));

        let mut addr_vec = vec::Vec::new();

        let mut i = 0;
        let mut test_closure = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![addr], &mut i);
            // check that tc remains unchanged
            addr_vec.push(*store_addr);
            false
        };

        scheduler
            .release_telecommands(&mut test_closure, &mut pool)
            .unwrap();

        let data = pool.read(&addr_vec[0]).unwrap();
        let check_tc = PusTc::from_bytes(&data).expect("incorrect Pus tc raw data");
        assert_eq!(check_tc.0, base_ping_tc_simple_ctor());
    }

    #[test]
    fn insert_wrapped_tc() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));

        let mut buf: [u8; 32] = [0; 32];
        let tc = scheduled_tc(UnixTimestamp::new_only_seconds(100), &mut buf);

        let addr = match scheduler
            .insert_wrapped_tc::<spacepackets::time::cds::TimeProvider>(&tc, &mut pool)
        {
            Ok(addr) => addr,
            Err(e) => {
                println!("{}", e);
                panic!();
            }
        };

        assert!(pool.has_element_at(&addr).unwrap());

        let data = pool.read(&addr).unwrap();
        let check_tc = PusTc::from_bytes(&data).expect("incorrect Pus tc raw data");
        assert_eq!(check_tc.0, base_ping_tc_simple_ctor());

        assert_eq!(scheduler.num_scheduled_telecommands(), 1);

        scheduler.update_time(UnixTimestamp::new_only_seconds(101));

        let mut addr_vec = vec::Vec::new();

        let mut i = 0;
        let mut test_closure = |boolvar: bool, store_addr: &StoreAddr| {
            common_check(boolvar, store_addr, vec![addr], &mut i);
            // check that tc remains unchanged
            addr_vec.push(*store_addr);
            false
        };

        scheduler
            .release_telecommands(&mut test_closure, &mut pool)
            .unwrap();

        let data = pool.read(&addr_vec[0]).unwrap();
        let check_tc = PusTc::from_bytes(&data).expect("incorrect Pus tc raw data");
        assert_eq!(check_tc.0, base_ping_tc_simple_ctor());
    }

    #[test]
    fn insert_wrong_service() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));

        let mut buf: [u8; 32] = [0; 32];
        let tc = wrong_tc_service(UnixTimestamp::new_only_seconds(100), &mut buf);

        match scheduler.insert_wrapped_tc::<spacepackets::time::cds::TimeProvider>(&tc, &mut pool) {
            Ok(_) => {
                panic!();
            }
            Err(e) => {
                if e != ScheduleError::WrongService {
                    panic!();
                }
            }
        }
    }

    #[test]
    fn insert_wrong_subservice() {
        let mut scheduler =
            PusScheduler::new(UnixTimestamp::new_only_seconds(0), Duration::from_secs(5));

        let mut pool = LocalPool::new(PoolCfg::new(vec![(10, 32), (5, 64)]));

        let mut buf: [u8; 32] = [0; 32];
        let tc = wrong_tc_subservice(UnixTimestamp::new_only_seconds(100), &mut buf);

        match scheduler.insert_wrapped_tc::<spacepackets::time::cds::TimeProvider>(&tc, &mut pool) {
            Ok(_) => {
                panic!();
            }
            Err(e) => {
                if e != ScheduleError::WrongSubservice {
                    panic!();
                }
            }
        }
    }
}
