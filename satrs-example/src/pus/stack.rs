use satrs::pus::EcssTcInMemConverter;

use super::{
    action::Pus8Wrapper, event::Pus5Wrapper, hk::Pus3Wrapper, scheduler::Pus11Wrapper,
    test::Service17CustomWrapper,
};

pub struct PusStack<TcInMemConverter: EcssTcInMemConverter> {
    event_srv: Pus5Wrapper<TcInMemConverter>,
    hk_srv: Pus3Wrapper<TcInMemConverter>,
    action_srv: Pus8Wrapper<TcInMemConverter>,
    schedule_srv: Pus11Wrapper<TcInMemConverter>,
    test_srv: Service17CustomWrapper<TcInMemConverter>,
}

impl<TcInMemConverter: EcssTcInMemConverter> PusStack<TcInMemConverter> {
    pub fn new(
        hk_srv: Pus3Wrapper<TcInMemConverter>,
        event_srv: Pus5Wrapper<TcInMemConverter>,
        action_srv: Pus8Wrapper<TcInMemConverter>,
        schedule_srv: Pus11Wrapper<TcInMemConverter>,
        test_srv: Service17CustomWrapper<TcInMemConverter>,
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
