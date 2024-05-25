use std::{sync::mpsc, time::Duration};

use asynchronix::{
    model::{Model, Output},
    time::Scheduler,
};
use satrs::power::SwitchStateBinary;
use satrs_minisim::{
    eps::{PcduReply, PcduSwitch, SwitchMapBinaryWrapper},
    SimReply,
};

pub const SWITCH_INFO_DELAY_MS: u64 = 10;

pub struct PcduModel {
    pub switcher_map: SwitchMapBinaryWrapper,
    pub mgm_switch: Output<SwitchStateBinary>,
    pub mgt_switch: Output<SwitchStateBinary>,
    pub reply_sender: mpsc::Sender<SimReply>,
}

impl PcduModel {
    pub fn new(reply_sender: mpsc::Sender<SimReply>) -> Self {
        Self {
            switcher_map: Default::default(),
            mgm_switch: Output::new(),
            mgt_switch: Output::new(),
            reply_sender,
        }
    }

    pub async fn request_switch_info(&mut self, _: (), scheduler: &Scheduler<Self>) {
        scheduler
            .schedule_event(
                Duration::from_millis(SWITCH_INFO_DELAY_MS),
                Self::send_switch_info,
                (),
            )
            .expect("requesting switch info failed");
    }

    pub fn send_switch_info(&mut self) {
        let reply = SimReply::new(&PcduReply::SwitchInfo(self.switcher_map.0.clone()));
        self.reply_sender.send(reply).unwrap();
    }

    pub async fn switch_device(
        &mut self,
        switch_and_target_state: (PcduSwitch, SwitchStateBinary),
    ) {
        let val = self
            .switcher_map
            .0
            .get_mut(&switch_and_target_state.0)
            .unwrap_or_else(|| panic!("switch {:?} not found", switch_and_target_state.0));
        *val = switch_and_target_state.1;
        match switch_and_target_state.0 {
            PcduSwitch::Mgm => {
                self.mgm_switch.send(switch_and_target_state.1).await;
            }
            PcduSwitch::Mgt => {
                self.mgt_switch.send(switch_and_target_state.1).await;
            }
        }
    }
}

impl Model for PcduModel {}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;

    use satrs_minisim::{
        eps::{PcduRequest, SwitchMapBinary},
        SerializableSimMsgPayload, SimComponent, SimMessageProvider, SimRequest,
    };

    use crate::test_helpers::SimTestbench;

    fn switch_device(
        sim_testbench: &mut SimTestbench,
        switch: PcduSwitch,
        target: SwitchStateBinary,
    ) {
        let request = SimRequest::new_with_epoch_time(PcduRequest::SwitchDevice {
            switch,
            state: target,
        });
        sim_testbench
            .send_request(request)
            .expect("sending MGM switch request failed");
        sim_testbench.handle_sim_requests_time_agnostic();
        sim_testbench.step();
    }

    #[allow(dead_code)]
    pub(crate) fn switch_device_off(sim_testbench: &mut SimTestbench, switch: PcduSwitch) {
        switch_device(sim_testbench, switch, SwitchStateBinary::Off);
    }
    pub(crate) fn switch_device_on(sim_testbench: &mut SimTestbench, switch: PcduSwitch) {
        switch_device(sim_testbench, switch, SwitchStateBinary::On);
    }

    pub(crate) fn get_all_off_switch_map() -> SwitchMapBinary {
        SwitchMapBinaryWrapper::default().0
    }

    fn check_switch_state(sim_testbench: &mut SimTestbench, expected_switch_map: &SwitchMapBinary) {
        let request = SimRequest::new_with_epoch_time(PcduRequest::RequestSwitchInfo);
        sim_testbench
            .send_request(request)
            .expect("sending MGM request failed");
        sim_testbench.handle_sim_requests_time_agnostic();
        sim_testbench.step();
        let sim_reply = sim_testbench.try_receive_next_reply();
        assert!(sim_reply.is_some());
        let sim_reply = sim_reply.unwrap();
        assert_eq!(sim_reply.component(), SimComponent::Pcdu);
        let pcdu_reply = PcduReply::from_sim_message(&sim_reply)
            .expect("failed to deserialize PCDU switch info");
        match pcdu_reply {
            PcduReply::SwitchInfo(switch_map) => {
                assert_eq!(switch_map, *expected_switch_map);
            }
        }
    }

    fn test_pcdu_switching_single_switch(switch: PcduSwitch, target: SwitchStateBinary) {
        let mut sim_testbench = SimTestbench::new();
        switch_device(&mut sim_testbench, switch, target);
        let mut switcher_map = get_all_off_switch_map();
        *switcher_map.get_mut(&switch).unwrap() = target;
        check_switch_state(&mut sim_testbench, &switcher_map);
    }

    #[test]
    fn test_pcdu_switcher_request() {
        let mut sim_testbench = SimTestbench::new();
        let request = SimRequest::new_with_epoch_time(PcduRequest::RequestSwitchInfo);
        sim_testbench
            .send_request(request)
            .expect("sending MGM request failed");
        sim_testbench.handle_sim_requests_time_agnostic();
        sim_testbench.step_by(Duration::from_millis(1));

        let sim_reply = sim_testbench.try_receive_next_reply();
        assert!(sim_reply.is_none());
        // Reply takes 20ms
        sim_testbench.step_by(Duration::from_millis(25));
        let sim_reply = sim_testbench.try_receive_next_reply();
        assert!(sim_reply.is_some());
        let sim_reply = sim_reply.unwrap();
        assert_eq!(sim_reply.component(), SimComponent::Pcdu);
        let pcdu_reply = PcduReply::from_sim_message(&sim_reply)
            .expect("failed to deserialize PCDU switch info");
        match pcdu_reply {
            PcduReply::SwitchInfo(switch_map) => {
                assert_eq!(switch_map, get_all_off_switch_map());
            }
        }
    }

    #[test]
    fn test_pcdu_switching_mgm_on() {
        test_pcdu_switching_single_switch(PcduSwitch::Mgm, SwitchStateBinary::On);
    }

    #[test]
    fn test_pcdu_switching_mgt_on() {
        test_pcdu_switching_single_switch(PcduSwitch::Mgt, SwitchStateBinary::On);
    }

    #[test]
    fn test_pcdu_switching_mgt_off() {
        test_pcdu_switching_single_switch(PcduSwitch::Mgt, SwitchStateBinary::On);
        test_pcdu_switching_single_switch(PcduSwitch::Mgt, SwitchStateBinary::Off);
    }
}
