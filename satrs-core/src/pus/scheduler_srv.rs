use super::scheduler::PusSchedulerInterface;
use super::{EcssTcInMemConverter, PusServiceBase, PusServiceHelper};
use crate::pool::PoolProvider;
use crate::pus::{PusPacketHandlerResult, PusPacketHandlingError};
use alloc::string::ToString;
use spacepackets::ecss::{scheduling, PusPacket};
use spacepackets::time::cds::TimeProvider;

/// This is a helper class for [std] environments to handle generic PUS 11 (scheduling service)
/// packets. This handler is able to handle the most important PUS requests for a scheduling
/// service which provides the [PusSchedulerInterface].
///
/// Please note that this class does not do the regular periodic handling like releasing any
/// telecommands inside the scheduler. The user can retrieve the wrapped scheduler via the
/// [Self::scheduler] and [Self::scheduler_mut] function and then use the scheduler API to release
/// telecommands when applicable.
pub struct PusService11SchedHandler<
    TcInMemConverter: EcssTcInMemConverter,
    Scheduler: PusSchedulerInterface,
> {
    pub service_helper: PusServiceHelper<TcInMemConverter>,
    scheduler: Scheduler,
}

impl<TcInMemConverter: EcssTcInMemConverter, Scheduler: PusSchedulerInterface>
    PusService11SchedHandler<TcInMemConverter, Scheduler>
{
    pub fn new(service_helper: PusServiceHelper<TcInMemConverter>, scheduler: Scheduler) -> Self {
        Self {
            service_helper,
            scheduler,
        }
    }

    pub fn scheduler_mut(&mut self) -> &mut Scheduler {
        &mut self.scheduler
    }

    pub fn scheduler(&self) -> &Scheduler {
        &self.scheduler
    }

    pub fn handle_one_tc(
        &mut self,
        sched_tc_pool: &mut (impl PoolProvider + ?Sized),
    ) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        let possible_packet = self.service_helper.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(PusPacketHandlerResult::Empty);
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        let tc = self
            .service_helper
            .tc_in_mem_converter
            .convert_ecss_tc_in_memory_to_reader(&ecss_tc_and_token.tc_in_memory)?;
        let subservice = PusPacket::subservice(&tc);
        let standard_subservice = scheduling::Subservice::try_from(subservice);
        if standard_subservice.is_err() {
            return Ok(PusPacketHandlerResult::CustomSubservice(
                subservice,
                ecss_tc_and_token.token,
            ));
        }
        let mut partial_error = None;
        let time_stamp = PusServiceBase::get_current_timestamp(&mut partial_error);
        match standard_subservice.unwrap() {
            scheduling::Subservice::TcEnableScheduling => {
                let start_token = self
                    .service_helper
                    .common
                    .verification_handler
                    .get_mut()
                    .start_success(ecss_tc_and_token.token, Some(&time_stamp))
                    .expect("Error sending start success");

                self.scheduler.enable();
                if self.scheduler.is_enabled() {
                    self.service_helper
                        .common
                        .verification_handler
                        .get_mut()
                        .completion_success(start_token, Some(&time_stamp))
                        .expect("Error sending completion success");
                } else {
                    return Err(PusPacketHandlingError::Other(
                        "failed to enabled scheduler".to_string(),
                    ));
                }
            }
            scheduling::Subservice::TcDisableScheduling => {
                let start_token = self
                    .service_helper
                    .common
                    .verification_handler
                    .get_mut()
                    .start_success(ecss_tc_and_token.token, Some(&time_stamp))
                    .expect("Error sending start success");

                self.scheduler.disable();
                if !self.scheduler.is_enabled() {
                    self.service_helper
                        .common
                        .verification_handler
                        .get_mut()
                        .completion_success(start_token, Some(&time_stamp))
                        .expect("Error sending completion success");
                } else {
                    return Err(PusPacketHandlingError::Other(
                        "failed to disable scheduler".to_string(),
                    ));
                }
            }
            scheduling::Subservice::TcResetScheduling => {
                let start_token = self
                    .service_helper
                    .common
                    .verification_handler
                    .get_mut()
                    .start_success(ecss_tc_and_token.token, Some(&time_stamp))
                    .expect("Error sending start success");

                self.scheduler
                    .reset(sched_tc_pool)
                    .expect("Error resetting TC Pool");

                self.service_helper
                    .common
                    .verification_handler
                    .get_mut()
                    .completion_success(start_token, Some(&time_stamp))
                    .expect("Error sending completion success");
            }
            scheduling::Subservice::TcInsertActivity => {
                let start_token = self
                    .service_helper
                    .common
                    .verification_handler
                    .get_mut()
                    .start_success(ecss_tc_and_token.token, Some(&time_stamp))
                    .expect("error sending start success");

                // let mut pool = self.sched_tc_pool.write().expect("locking pool failed");
                self.scheduler
                    .insert_wrapped_tc::<TimeProvider>(&tc, sched_tc_pool)
                    .expect("insertion of activity into pool failed");

                self.service_helper
                    .common
                    .verification_handler
                    .get_mut()
                    .completion_success(start_token, Some(&time_stamp))
                    .expect("sending completion success failed");
            }
            _ => {
                // Treat unhandled standard subservices as custom subservices for now.
                return Ok(PusPacketHandlerResult::CustomSubservice(
                    subservice,
                    ecss_tc_and_token.token,
                ));
            }
        }
        if let Some(partial_error) = partial_error {
            return Ok(PusPacketHandlerResult::RequestHandledPartialSuccess(
                partial_error,
            ));
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}

#[cfg(test)]
mod tests {
    use crate::pool::{StaticMemoryPool, StaticPoolConfig};
    use crate::pus::tests::TEST_APID;
    use crate::pus::{
        scheduler::{self, PusSchedulerInterface, TcInfo},
        tests::{PusServiceHandlerWithSharedStoreCommon, PusTestHarness},
        verification::{RequestId, TcStateAccepted, VerificationToken},
        EcssTcInSharedStoreConverter,
    };
    use alloc::collections::VecDeque;
    use delegate::delegate;
    use spacepackets::ecss::scheduling::Subservice;
    use spacepackets::ecss::tc::PusTcSecondaryHeader;
    use spacepackets::ecss::WritablePusPacket;
    use spacepackets::time::TimeWriter;
    use spacepackets::SpHeader;
    use spacepackets::{
        ecss::{tc::PusTcCreator, tm::PusTmReader},
        time::cds,
    };

    use super::PusService11SchedHandler;

    struct Pus11HandlerWithStoreTester {
        common: PusServiceHandlerWithSharedStoreCommon,
        handler: PusService11SchedHandler<EcssTcInSharedStoreConverter, TestScheduler>,
        sched_tc_pool: StaticMemoryPool,
    }

    impl Pus11HandlerWithStoreTester {
        pub fn new() -> Self {
            let test_scheduler = TestScheduler::default();
            let pool_cfg = StaticPoolConfig::new(alloc::vec![(16, 16), (8, 32), (4, 64)]);
            let sched_tc_pool = StaticMemoryPool::new(pool_cfg.clone());
            let (common, srv_handler) = PusServiceHandlerWithSharedStoreCommon::new();
            Self {
                common,
                handler: PusService11SchedHandler::new(srv_handler, test_scheduler),
                sched_tc_pool,
            }
        }
    }

    impl PusTestHarness for Pus11HandlerWithStoreTester {
        delegate! {
            to self.common {
                fn send_tc(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted>;
                fn read_next_tm(&mut self) -> PusTmReader<'_>;
                fn check_no_tm_available(&self) -> bool;
                fn check_next_verification_tm(&self, subservice: u8, expected_request_id: RequestId);
            }
        }
    }

    #[derive(Default)]
    pub struct TestScheduler {
        reset_count: u32,
        enabled: bool,
        enabled_count: u32,
        disabled_count: u32,
        inserted_tcs: VecDeque<TcInfo>,
    }

    impl PusSchedulerInterface for TestScheduler {
        type TimeProvider = cds::TimeProvider;

        fn reset(
            &mut self,
            _store: &mut (impl crate::pool::PoolProvider + ?Sized),
        ) -> Result<(), crate::pool::StoreError> {
            self.reset_count += 1;
            Ok(())
        }

        fn is_enabled(&self) -> bool {
            self.enabled
        }

        fn enable(&mut self) {
            self.enabled_count += 1;
            self.enabled = true;
        }

        fn disable(&mut self) {
            self.disabled_count += 1;
            self.enabled = false;
        }

        fn insert_unwrapped_and_stored_tc(
            &mut self,
            _time_stamp: spacepackets::time::UnixTimestamp,
            info: crate::pus::scheduler::TcInfo,
        ) -> Result<(), crate::pus::scheduler::ScheduleError> {
            self.inserted_tcs.push_back(info);
            Ok(())
        }
    }

    fn generic_subservice_send(
        test_harness: &mut Pus11HandlerWithStoreTester,
        subservice: Subservice,
    ) {
        let mut reply_header = SpHeader::tm_unseg(TEST_APID, 0, 0).unwrap();
        let tc_header = PusTcSecondaryHeader::new_simple(11, subservice as u8);
        let enable_scheduling = PusTcCreator::new(&mut reply_header, tc_header, &[0; 7], true);
        let token = test_harness.send_tc(&enable_scheduling);

        let request_id = token.req_id();
        test_harness
            .handler
            .handle_one_tc(&mut test_harness.sched_tc_pool)
            .unwrap();
        test_harness.check_next_verification_tm(1, request_id);
        test_harness.check_next_verification_tm(3, request_id);
        test_harness.check_next_verification_tm(7, request_id);
    }

    #[test]
    fn test_scheduling_enabling_tc() {
        let mut test_harness = Pus11HandlerWithStoreTester::new();
        test_harness.handler.scheduler_mut().disable();
        assert!(!test_harness.handler.scheduler().is_enabled());
        generic_subservice_send(&mut test_harness, Subservice::TcEnableScheduling);
        assert!(test_harness.handler.scheduler().is_enabled());
        assert_eq!(test_harness.handler.scheduler().enabled_count, 1);
    }

    #[test]
    fn test_scheduling_disabling_tc() {
        let mut test_harness = Pus11HandlerWithStoreTester::new();
        test_harness.handler.scheduler_mut().enable();
        assert!(test_harness.handler.scheduler().is_enabled());
        generic_subservice_send(&mut test_harness, Subservice::TcDisableScheduling);
        assert!(!test_harness.handler.scheduler().is_enabled());
        assert_eq!(test_harness.handler.scheduler().disabled_count, 1);
    }

    #[test]
    fn test_reset_scheduler_tc() {
        let mut test_harness = Pus11HandlerWithStoreTester::new();
        generic_subservice_send(&mut test_harness, Subservice::TcResetScheduling);
        assert_eq!(test_harness.handler.scheduler().reset_count, 1);
    }

    #[test]
    fn test_insert_activity_tc() {
        let mut test_harness = Pus11HandlerWithStoreTester::new();
        let mut reply_header = SpHeader::tm_unseg(TEST_APID, 0, 0).unwrap();
        let mut sec_header = PusTcSecondaryHeader::new_simple(17, 1);
        let ping_tc = PusTcCreator::new(&mut reply_header, sec_header, &[], true);
        let req_id_ping_tc = scheduler::RequestId::from_tc(&ping_tc);
        let stamper = cds::TimeProvider::from_now_with_u16_days().expect("time provider failed");
        let mut sched_app_data: [u8; 64] = [0; 64];
        let mut written_len = stamper.write_to_bytes(&mut sched_app_data).unwrap();
        let ping_raw = ping_tc.to_vec().expect("generating raw tc failed");
        sched_app_data[written_len..written_len + ping_raw.len()].copy_from_slice(&ping_raw);
        written_len += ping_raw.len();
        reply_header = SpHeader::tm_unseg(TEST_APID, 1, 0).unwrap();
        sec_header = PusTcSecondaryHeader::new_simple(11, Subservice::TcInsertActivity as u8);
        let enable_scheduling = PusTcCreator::new(
            &mut reply_header,
            sec_header,
            &sched_app_data[..written_len],
            true,
        );
        let token = test_harness.send_tc(&enable_scheduling);

        let request_id = token.req_id();
        test_harness
            .handler
            .handle_one_tc(&mut test_harness.sched_tc_pool)
            .unwrap();
        test_harness.check_next_verification_tm(1, request_id);
        test_harness.check_next_verification_tm(3, request_id);
        test_harness.check_next_verification_tm(7, request_id);
        let tc_info = test_harness
            .handler
            .scheduler_mut()
            .inserted_tcs
            .pop_front()
            .unwrap();
        assert_eq!(tc_info.request_id(), req_id_ping_tc);
    }
}
