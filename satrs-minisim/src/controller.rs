use std::{sync::mpsc, time::Duration};

use asynchronix::{
    simulation::{Address, Simulation},
    time::{Clock, MonotonicTime, SystemClock},
};
use satrs_minisim::{
    acs::{lis3mdl::MgmLis3MdlReply, MgmRequestLis3Mdl, MgtRequest},
    eps::PcduRequest,
    SerializableSimMsgPayload, SimComponent, SimCtrlReply, SimCtrlRequest, SimMessageProvider,
    SimReply, SimRequest, SimRequestError,
};

use crate::{
    acs::{MagnetometerModel, MagnetorquerModel},
    eps::PcduModel,
};

const SIM_CTRL_REQ_WIRETAPPING: bool = true;
const MGM_REQ_WIRETAPPING: bool = true;
const PCDU_REQ_WIRETAPPING: bool = true;
const MGT_REQ_WIRETAPPING: bool = true;

// The simulation controller processes requests and drives the simulation.
pub struct SimController {
    pub sys_clock: SystemClock,
    pub request_receiver: mpsc::Receiver<SimRequest>,
    pub reply_sender: mpsc::Sender<SimReply>,
    pub simulation: Simulation,
    pub mgm_addr: Address<MagnetometerModel<MgmLis3MdlReply>>,
    pub pcdu_addr: Address<PcduModel>,
    pub mgt_addr: Address<MagnetorquerModel>,
}

impl SimController {
    pub fn new(
        sys_clock: SystemClock,
        request_receiver: mpsc::Receiver<SimRequest>,
        reply_sender: mpsc::Sender<SimReply>,
        simulation: Simulation,
        mgm_addr: Address<MagnetometerModel<MgmLis3MdlReply>>,
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
            self.handle_sim_requests(t_old);
            self.simulation
                .step_until(t)
                .expect("simulation step failed");
        }
    }

    pub fn handle_sim_requests(&mut self, old_timestamp: MonotonicTime) {
        loop {
            match self.request_receiver.try_recv() {
                Ok(request) => {
                    if request.timestamp < old_timestamp {
                        log::warn!("stale data with timestamp {:?} received", request.timestamp);
                    }
                    if let Err(e) = match request.component() {
                        SimComponent::SimCtrl => self.handle_ctrl_request(&request),
                        SimComponent::MgmLis3Mdl => self.handle_mgm_request(&request),
                        SimComponent::Mgt => self.handle_mgt_request(&request),
                        SimComponent::Pcdu => self.handle_pcdu_request(&request),
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
        if SIM_CTRL_REQ_WIRETAPPING {
            log::info!("received sim ctrl request: {:?}", sim_ctrl_request);
        }
        match sim_ctrl_request {
            SimCtrlRequest::Ping => {
                self.reply_sender
                    .send(SimReply::new(&SimCtrlReply::Pong))
                    .expect("sending reply from sim controller failed");
            }
        }
        Ok(())
    }

    fn handle_mgm_request(&mut self, request: &SimRequest) -> Result<(), SimRequestError> {
        let mgm_request = MgmRequestLis3Mdl::from_sim_message(request)?;
        if MGM_REQ_WIRETAPPING {
            log::info!("received MGM request: {:?}", mgm_request);
        }
        match mgm_request {
            MgmRequestLis3Mdl::RequestSensorData => {
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
        if PCDU_REQ_WIRETAPPING {
            log::info!("received PCDU request: {:?}", pcdu_request);
        }
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
        if MGT_REQ_WIRETAPPING {
            log::info!("received MGT request: {:?}", mgt_request);
        }
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
            request.component(),
            error
        );
        self.reply_sender
            .send(SimReply::new(&SimCtrlReply::from(error)))
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
        let request = SimRequest::new_with_epoch_time(SimCtrlRequest::Ping);
        sim_testbench
            .send_request(request)
            .expect("sending sim ctrl request failed");
        sim_testbench.handle_sim_requests_time_agnostic();
        sim_testbench.step();
        let sim_reply = sim_testbench.try_receive_next_reply();
        assert!(sim_reply.is_some());
        let sim_reply = sim_reply.unwrap();
        assert_eq!(sim_reply.component(), SimComponent::SimCtrl);
        let reply = SimCtrlReply::from_sim_message(&sim_reply)
            .expect("failed to deserialize MGM sensor values");
        assert_eq!(reply, SimCtrlReply::Pong);
    }
}
