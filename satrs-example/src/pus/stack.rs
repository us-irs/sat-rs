use satrs::{
    pus::{
        verification::VerificationReportingProvider, EcssTcInMemConverter, EcssTcReceiverCore,
        EcssTmSenderCore,
    },
    spacepackets::time::{cds, TimeWriter},
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
    hk_srv_wrapper: Pus3Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    action_srv_wrapper: Pus8Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
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
            action_srv_wrapper: action_srv,
            schedule_srv,
            test_srv,
            hk_srv_wrapper: hk_srv,
        }
    }

    pub fn periodic_operation(&mut self) {
        // Release all telecommands which reached their release time before calling the service
        // handlers.
        self.schedule_srv.release_tcs();
        let time_stamp = cds::TimeProvider::from_now_with_u16_days()
            .expect("time stamp generation error")
            .to_vec()
            .unwrap();
        loop {
            let mut nothing_to_do = true;
            let mut is_srv_finished =
                |tc_handling_done: bool, reply_handling_done: Option<bool>| {
                    if !tc_handling_done
                        || (reply_handling_done.is_some() && !reply_handling_done.unwrap())
                    {
                        nothing_to_do = false;
                    }
                };
            is_srv_finished(self.test_srv.poll_and_handle_next_packet(&time_stamp), None);
            is_srv_finished(self.schedule_srv.poll_and_handle_next_tc(&time_stamp), None);
            is_srv_finished(self.event_srv.poll_and_handle_next_tc(&time_stamp), None);
            is_srv_finished(
                self.action_srv_wrapper.poll_and_handle_next_tc(&time_stamp),
                Some(
                    self.action_srv_wrapper
                        .poll_and_handle_next_reply(&time_stamp),
                ),
            );
            is_srv_finished(
                self.hk_srv_wrapper.poll_and_handle_next_tc(&time_stamp),
                Some(self.hk_srv_wrapper.poll_and_handle_next_reply(&time_stamp)),
            );
            if nothing_to_do {
                // Timeout checking is only done once.
                self.action_srv_wrapper.service.check_for_request_timeouts();
                self.hk_srv_wrapper.service.check_for_request_timeouts();
                break;
            }
        }
    }
}
