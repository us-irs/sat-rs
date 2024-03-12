use std::{sync::mpsc, time::Duration};

use asynchronix::{
    simulation::{Address, Simulation},
    time::{Clock, MonotonicTime, SystemClock},
};
use satrs_minisim::{
    acs::{MgmRequest, MgtRequest},
    eps::PcduRequest,
    SerializableSimMsgPayload, SimCtrlReply, SimCtrlRequest, SimMessageProvider, SimReply,
    SimRequest, SimRequestError, SimTarget,
};

use crate::{
    acs::{MagnetometerModel, MagnetorquerModel},
    eps::PcduModel,
};

// The simulation controller processes requests and drives the simulation.
pub struct SimController {
    pub sys_clock: SystemClock,
    pub request_receiver: mpsc::Receiver<SimRequest>,
    pub reply_sender: mpsc::Sender<SimReply>,
    pub simulation: Simulation,
    pub mgm_addr: Address<MagnetometerModel>,
    pub pcdu_addr: Address<PcduModel>,
    pub mgt_addr: Address<MagnetorquerModel>,
}

impl SimController {
    pub fn new(
        sys_clock: SystemClock,
        request_receiver: mpsc::Receiver<SimRequest>,
        reply_sender: mpsc::Sender<SimReply>,
        simulation: Simulation,
        mgm_addr: Address<MagnetometerModel>,
        pcdu_addr: Address<PcduModel>,
        mgt_addr: Address<MagnetorquerModel>,
    ) -> Self {
        Self {
            sys_clock,
            request_receiver,
            reply_sender,
            simulation,
            mgm_addr,
            pcdu_addr,
            mgt_addr,
        }
    }

    pub fn run(&mut self, start_time: MonotonicTime, udp_polling_interval_ms: u64) {
        let mut t = start_time;
        loop {
            let t_old = t;
            // Check for UDP requests every millisecond. Shift the simulator ahead here to prevent
            // replies lying in the past.
            t += Duration::from_millis(udp_polling_interval_ms);
            self.sys_clock.synchronize(t);
            self.handle_sim_requests();
            self.simulation
                .step_until(t)
                .expect("simulation step failed");

        }
    }

    pub fn handle_sim_requests(&mut self) {
        loop {
            match self.request_receiver.try_recv() {
                Ok(request) => {
                    if let Err(e) = match request.target() {
                        SimTarget::SimCtrl => self.handle_ctrl_request(&request),
                        SimTarget::Mgm => self.handle_mgm_request(&request),
                        SimTarget::Mgt => self.handle_mgt_request(&request),
                        SimTarget::Pcdu => self.handle_pcdu_request(&request),
                    } {
                        self.handle_invalid_request_with_valid_target(e, &request)
                    }
                }
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => break,
                    mpsc::TryRecvError::Disconnected => {
                        panic!("all request sender disconnected")
                    }
                },
            }
        }
    }

    fn handle_ctrl_request(&mut self, request: &SimRequest) -> Result<(), SimRequestError> {
        let sim_ctrl_request = SimCtrlRequest::from_sim_message(request)?;
        match sim_ctrl_request {
            SimCtrlRequest::Ping => {
                self.reply_sender
                    .send(SimReply::new(SimCtrlReply::Pong))
                    .expect("sending reply from sim controller failed");
            }
        }
        Ok(())
    }
    fn handle_mgm_request(&mut self, request: &SimRequest) -> Result<(), SimRequestError> {
        let mgm_request = MgmRequest::from_sim_message(request)?;
        match mgm_request {
            MgmRequest::RequestSensorData => {
                self.simulation.send_event(
                    MagnetometerModel::send_sensor_values,
                    (),
                    &self.mgm_addr,
                );
            }
        }
        Ok(())
    }

    fn handle_pcdu_request(&mut self, request: &SimRequest) -> Result<(), SimRequestError> {
        let pcdu_request = PcduRequest::from_sim_message(request)?;
        match pcdu_request {
            PcduRequest::RequestSwitchInfo => {
                self.simulation
                    .send_event(PcduModel::request_switch_info, (), &self.pcdu_addr);
            }
            PcduRequest::SwitchDevice { switch, state } => {
                self.simulation.send_event(
                    PcduModel::switch_device,
                    (switch, state),
                    &self.pcdu_addr,
                );
            }
        }
        Ok(())
    }

    fn handle_mgt_request(&mut self, request: &SimRequest) -> Result<(), SimRequestError> {
        let mgt_request = MgtRequest::from_sim_message(request)?;
        match mgt_request {
            MgtRequest::ApplyTorque { duration, dipole } => self.simulation.send_event(
                MagnetorquerModel::apply_torque,
                (duration, dipole),
                &self.mgt_addr,
            ),
            MgtRequest::RequestHk => self.simulation.send_event(
                MagnetorquerModel::request_housekeeping_data,
                (),
                &self.mgt_addr,
            ),
        }
        Ok(())
    }

    fn handle_invalid_request_with_valid_target(
        &self,
        error: SimRequestError,
        request: &SimRequest,
    ) {
        log::warn!(
            "received invalid {:?} request: {:?}",
            request.target(),
            error
        );
        self.reply_sender
            .send(SimReply::new(SimCtrlReply::from(error)))
            .expect("sending reply from sim controller failed");
    }
}

#[cfg(test)]
mod tests {
    use crate::test_helpers::SimTestbench;

    use super::*;

    #[test]
    fn test_basic_ping() {
        let mut sim_testbench = SimTestbench::new();
        let request = SimRequest::new(SimCtrlRequest::Ping);
        sim_testbench
            .send_request(request)
            .expect("sending sim ctrl request failed");
        sim_testbench.handle_sim_requests();
        sim_testbench.step();
        let sim_reply = sim_testbench.try_receive_next_reply();
        assert!(sim_reply.is_some());
        let sim_reply = sim_reply.unwrap();
        assert_eq!(sim_reply.target(), SimTarget::SimCtrl);
        let reply = SimCtrlReply::from_sim_message(&sim_reply)
            .expect("failed to deserialize MGM sensor values");
        assert_eq!(reply, SimCtrlReply::Pong);
    }
}
