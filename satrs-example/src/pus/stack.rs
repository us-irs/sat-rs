use crate::pus::mode::ModeServiceWrapper;
use derive_new::new;
use satrs::{
    pus::{EcssTcInMemConverter, EcssTmSender},
    spacepackets::time::{cds, TimeWriter},
};

use super::{
    action::ActionServiceWrapper, event::EventServiceWrapper, hk::HkServiceWrapper,
    scheduler::SchedulingServiceWrapper, test::TestCustomServiceWrapper, HandlingStatus,
    TargetedPusService,
};

#[derive(new)]
pub struct PusStack<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter> {
    test_srv: TestCustomServiceWrapper<TmSender, TcInMemConverter>,
    hk_srv_wrapper: HkServiceWrapper<TmSender, TcInMemConverter>,
    event_srv: EventServiceWrapper<TmSender, TcInMemConverter>,
    action_srv_wrapper: ActionServiceWrapper<TmSender, TcInMemConverter>,
    schedule_srv: SchedulingServiceWrapper<TmSender, TcInMemConverter>,
    mode_srv: ModeServiceWrapper<TmSender, TcInMemConverter>,
}

impl<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter>
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
                |_srv_id: u8,
                 tc_handling_status: HandlingStatus,
                 reply_handling_status: Option<HandlingStatus>| {
                    if tc_handling_status == HandlingStatus::HandledOne
                        || (reply_handling_status.is_some()
                            && reply_handling_status.unwrap() == HandlingStatus::HandledOne)
                    {
                        nothing_to_do = false;
                    }
                };
            is_srv_finished(
                17,
                self.test_srv.poll_and_handle_next_packet(&time_stamp),
                None,
            );
            is_srv_finished(
                11,
                self.schedule_srv.poll_and_handle_next_tc(&time_stamp),
                None,
            );
            is_srv_finished(5, self.event_srv.poll_and_handle_next_tc(&time_stamp), None);
            is_srv_finished(
                8,
                self.action_srv_wrapper.poll_and_handle_next_tc(&time_stamp),
                Some(
                    self.action_srv_wrapper
                        .poll_and_handle_next_reply(&time_stamp),
                ),
            );
            is_srv_finished(
                3,
                self.hk_srv_wrapper.poll_and_handle_next_tc(&time_stamp),
                Some(self.hk_srv_wrapper.poll_and_handle_next_reply(&time_stamp)),
            );
            is_srv_finished(
                200,
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
