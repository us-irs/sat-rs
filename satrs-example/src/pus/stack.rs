use crate::pus::mode::ModeServiceWrapper;
use derive_new::new;
use satrs::{
    pus::{EcssTcInMemConverter, EcssTmSenderCore},
    spacepackets::time::{cds, TimeWriter},
};

use super::{
    action::ActionServiceWrapper, event::EventServiceWrapper, hk::HkServiceWrapper,
    scheduler::SchedulingServiceWrapper, test::TestCustomServiceWrapper, HandlingStatus,
    TargetedPusService,
};

#[derive(new)]
pub struct PusStack<TmSender: EcssTmSenderCore, TcInMemConverter: EcssTcInMemConverter> {
    test_srv: TestCustomServiceWrapper<TmSender, TcInMemConverter>,
    hk_srv_wrapper: HkServiceWrapper<TmSender, TcInMemConverter>,
    event_srv: EventServiceWrapper<TmSender, TcInMemConverter>,
    action_srv_wrapper: ActionServiceWrapper<TmSender, TcInMemConverter>,
    schedule_srv: SchedulingServiceWrapper<TmSender, TcInMemConverter>,
    mode_srv: ModeServiceWrapper<TmSender, TcInMemConverter>,
}

impl<TmSender: EcssTmSenderCore, TcInMemConverter: EcssTcInMemConverter>
    PusStack<TmSender, TcInMemConverter>
{
    pub fn periodic_operation(&mut self) {
        // Release all telecommands which reached their release time before calling the service
        // handlers.
        self.schedule_srv.release_tcs();
        let time_stamp = cds::CdsTime::now_with_u16_days()
            .expect("time stamp generation error")
            .to_vec()
            .unwrap();
        loop {
            let mut nothing_to_do = true;
            let mut is_srv_finished =
                |tc_handling_done: bool, reply_handling_done: Option<HandlingStatus>| {
                    if !tc_handling_done
                        || (reply_handling_done.is_some()
                            && reply_handling_done.unwrap() == HandlingStatus::Empty)
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
            is_srv_finished(
                self.mode_srv.poll_and_handle_next_tc(&time_stamp),
                Some(self.mode_srv.poll_and_handle_next_reply(&time_stamp)),
            );
            if nothing_to_do {
                // Timeout checking is only done once.
                self.action_srv_wrapper.check_for_request_timeouts();
                self.hk_srv_wrapper.check_for_request_timeouts();
                self.mode_srv.check_for_request_timeouts();
                break;
            }
        }
    }
}
