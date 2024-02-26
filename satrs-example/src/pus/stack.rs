use satrs::pus::{
    verification::VerificationReportingProvider, EcssTcInMemConverter, EcssTcReceiverCore,
    EcssTmSenderCore,
};

use super::{
    action::Pus8Wrapper, event::Pus5Wrapper, hk::Pus3Wrapper, scheduler::Pus11Wrapper,
    test::Service17CustomWrapper,
};

pub struct PusStack<
    TcReceiver: EcssTcReceiverCore,
    TmSender: EcssTmSenderCore,
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    event_srv: Pus5Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    hk_srv: Pus3Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    action_srv: Pus8Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    schedule_srv: Pus11Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    test_srv: Service17CustomWrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
}

impl<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > PusStack<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>
{
    pub fn new(
        hk_srv: Pus3Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
        event_srv: Pus5Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
        action_srv: Pus8Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
        schedule_srv: Pus11Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
        test_srv: Service17CustomWrapper<
            TcReceiver,
            TmSender,
            TcInMemConverter,
            VerificationReporter,
        >,
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
