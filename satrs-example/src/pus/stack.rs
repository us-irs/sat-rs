use satrs::pus::{verification::VerificationReportingProvider, EcssTcInMemConverter};

use super::{
    action::Pus8Wrapper, event::Pus5Wrapper, hk::Pus3Wrapper, scheduler::Pus11Wrapper,
    test::Service17CustomWrapper,
};

pub struct PusStack<
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    event_srv: Pus5Wrapper<TcInMemConverter, VerificationReporter>,
    hk_srv: Pus3Wrapper<TcInMemConverter, VerificationReporter>,
    action_srv: Pus8Wrapper<TcInMemConverter, VerificationReporter>,
    schedule_srv: Pus11Wrapper<TcInMemConverter, VerificationReporter>,
    test_srv: Service17CustomWrapper<TcInMemConverter, VerificationReporter>,
}

impl<
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > PusStack<TcInMemConverter, VerificationReporter>
{
    pub fn new(
        hk_srv: Pus3Wrapper<TcInMemConverter, VerificationReporter>,
        event_srv: Pus5Wrapper<TcInMemConverter, VerificationReporter>,
        action_srv: Pus8Wrapper<TcInMemConverter, VerificationReporter>,
        schedule_srv: Pus11Wrapper<TcInMemConverter, VerificationReporter>,
        test_srv: Service17CustomWrapper<TcInMemConverter, VerificationReporter>,
    ) -> Self {
        Self {
            event_srv,
            action_srv,
            schedule_srv,
            test_srv,
            hk_srv,
        }
    }

    pub fn periodic_operation(&mut self) {
        self.schedule_srv.release_tcs();
        loop {
            let mut all_queues_empty = true;
            let mut is_srv_finished = |srv_handler_finished: bool| {
                if !srv_handler_finished {
                    all_queues_empty = false;
                }
            };
            is_srv_finished(self.test_srv.handle_next_packet());
            is_srv_finished(self.schedule_srv.handle_next_packet());
            is_srv_finished(self.event_srv.handle_next_packet());
            is_srv_finished(self.action_srv.handle_next_packet());
            is_srv_finished(self.hk_srv.handle_next_packet());
            if all_queues_empty {
                break;
            }
        }
    }
}
