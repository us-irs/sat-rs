use std::{f32::consts::PI, sync::mpsc, time::Duration};

use asynchronix::{
    model::{Model, Output},
    time::Scheduler,
};
use satrs::power::SwitchStateBinary;
use satrs_minisim::{
    acs::{MgmReply, MgmSensorValues, MgtDipole, MgtHkSet, MgtReply, MGT_GEN_MAGNETIC_FIELD},
    SimReply, SimTarget,
};

use crate::time::current_millis;

// Earth magnetic field varies between -30 uT and 30 uT
const AMPLITUDE_MGM: f32 = 0.03;
// Lets start with a simple frequency here.
const FREQUENCY_MGM: f32 = 1.0;
const PHASE_X: f32 = 0.0;
// Different phases to have different values on the other axes.
const PHASE_Y: f32 = 0.1;
const PHASE_Z: f32 = 0.2;

/// Simple model for a magnetometer where the measure magnetic fields are modeled with sine waves.
///
/// Please note that that a more realistic MGM model wouold include the following components
/// which are not included here to simplify the model:
///
/// 1. It would probably generate signed [i16] values which need to be converted to SI units
///    because it is a digital sensor
/// 2. It would sample the magnetic field at a high fixed rate. This might not be possible for
///    a general purpose OS, but self self-sampling at a relatively high rate (20-40 ms) might
///    stil lbe possible.
pub struct MagnetometerModel {
    pub switch_state: SwitchStateBinary,
    pub periodicity: Duration,
    pub external_mag_field: Option<MgmSensorValues>,
    pub reply_sender: mpsc::Sender<SimReply>,
}

impl MagnetometerModel {
    pub fn new(periodicity: Duration, reply_sender: mpsc::Sender<SimReply>) -> Self {
        Self {
            switch_state: SwitchStateBinary::Off,
            periodicity,
            external_mag_field: None,
            reply_sender,
        }
    }

    pub async fn switch_device(&mut self, switch_state: SwitchStateBinary) {
        self.switch_state = switch_state;
    }

    pub async fn send_sensor_values(&mut self, _: (), scheduler: &Scheduler<Self>) {
        self.reply_sender
            .send(SimReply::new(
                SimTarget::Mgm,
                MgmReply {
                    switch_state: self.switch_state,
                    sensor_values: self
                        .calculate_current_mgm_tuple(current_millis(scheduler.time())),
                },
            ))
            .expect("sending MGM sensor values failed");
    }

    // Devices like magnetorquers generate a strong magnetic field which overrides the default
    // model for the measured magnetic field.
    pub async fn apply_external_magnetic_field(&mut self, field: MgmSensorValues) {
        self.external_mag_field = Some(field);
    }

    fn calculate_current_mgm_tuple(&self, time_ms: u64) -> MgmSensorValues {
        if SwitchStateBinary::On == self.switch_state {
            if let Some(ext_field) = self.external_mag_field {
                return ext_field;
            }
            let base_sin_val = 2.0 * PI * FREQUENCY_MGM * (time_ms as f32 / 1000.0);
            return MgmSensorValues {
                x: AMPLITUDE_MGM * (base_sin_val + PHASE_X).sin(),
                y: AMPLITUDE_MGM * (base_sin_val + PHASE_Y).sin(),
                z: AMPLITUDE_MGM * (base_sin_val + PHASE_Z).sin(),
            };
        }
        MgmSensorValues {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

impl Model for MagnetometerModel {}

pub struct MagnetorquerModel {
    switch_state: SwitchStateBinary,
    torquing: bool,
    torque_dipole: MgtDipole,
    pub gen_magnetic_field: Output<MgmSensorValues>,
    reply_sender: mpsc::Sender<SimReply>,
}

impl MagnetorquerModel {
    pub fn new(reply_sender: mpsc::Sender<SimReply>) -> Self {
        Self {
            switch_state: SwitchStateBinary::Off,
            torquing: false,
            torque_dipole: MgtDipole::default(),
            gen_magnetic_field: Output::new(),
            reply_sender,
        }
    }

    pub async fn apply_torque(
        &mut self,
        duration_and_dipole: (Duration, MgtDipole),
        scheduler: &Scheduler<Self>,
    ) {
        self.torque_dipole = duration_and_dipole.1;
        self.torquing = true;
        if scheduler
            .schedule_event(duration_and_dipole.0, Self::clear_torque, ())
            .is_err()
        {
            log::warn!("torque clearing can only be set for a future time.");
        }
        self.generate_magnetic_field(()).await;
    }

    pub async fn clear_torque(&mut self, _: ()) {
        self.torque_dipole = MgtDipole::default();
        self.torquing = false;
        self.generate_magnetic_field(()).await;
    }

    pub async fn switch_device(&mut self, switch_state: SwitchStateBinary) {
        self.switch_state = switch_state;
        self.generate_magnetic_field(()).await;
    }

    pub async fn request_housekeeping_data(&mut self, _: (), scheduler: &Scheduler<Self>) {
        if self.switch_state != SwitchStateBinary::On {
            return;
        }
        scheduler
            .schedule_event(Duration::from_millis(15), Self::send_housekeeping_data, ())
            .expect("requesting housekeeping data failed")
    }

    pub fn send_housekeeping_data(&mut self) {
        let mgt_reply = MgtReply::Hk(MgtHkSet {
            dipole: self.torque_dipole,
            torquing: self.torquing,
        });
        self.reply_sender
            .send(SimReply::new(SimTarget::Mgt, mgt_reply))
            .unwrap();
    }

    fn calc_magnetic_field(&self, _: MgtDipole) -> MgmSensorValues {
        // Simplified model: Just returns some fixed magnetic field for now.
        // Later, we could make this more fancy by incorporating the commanded dipole.
        MGT_GEN_MAGNETIC_FIELD
    }

    /// A torquing magnetorquer generates a magnetic field. This function can be used to apply
    /// the magnetic field.
    async fn generate_magnetic_field(&mut self, _: ()) {
        if self.switch_state != SwitchStateBinary::On || !self.torquing {
            return;
        }
        self.gen_magnetic_field
            .send(self.calc_magnetic_field(self.torque_dipole))
            .await;
    }
}

impl Model for MagnetorquerModel {}

#[cfg(test)]
pub mod tests {
    use std::time::Duration;

    use satrs::power::SwitchStateBinary;
    use satrs_minisim::{
        acs::{MgmReply, MgmRequest, MgtDipole, MgtHkSet, MgtReply, MgtRequest},
        eps::PcduSwitch,
        SimRequest, SimTarget,
    };

    use crate::{eps::tests::switch_device_on, test_helpers::SimTestbench};

    #[test]
    fn test_basic_mgm_request() {
        let mut sim_testbench = SimTestbench::new();
        let mgm_request = MgmRequest::RequestSensorData;
        let request = SimRequest::new(SimTarget::Mgm, mgm_request);
        sim_testbench
            .send_request(request)
            .expect("sending MGM request failed");
        sim_testbench.handle_sim_requests();
        sim_testbench.step();
        let sim_reply = sim_testbench.try_receive_next_reply();
        assert!(sim_reply.is_some());
        let sim_reply = sim_reply.unwrap();
        assert_eq!(sim_reply.target(), SimTarget::Mgm);
        let reply: MgmReply = serde_json::from_str(sim_reply.reply())
            .expect("failed to deserialize MGM sensor values");
        assert_eq!(reply.switch_state, SwitchStateBinary::Off);
        assert_eq!(reply.sensor_values.x, 0.0);
        assert_eq!(reply.sensor_values.y, 0.0);
        assert_eq!(reply.sensor_values.z, 0.0);
    }

    #[test]
    fn test_basic_mgm_request_switched_on() {
        let mut sim_testbench = SimTestbench::new();
        switch_device_on(&mut sim_testbench, PcduSwitch::Mgm);

        let mgm_request = MgmRequest::RequestSensorData;
        let mut request = SimRequest::new(SimTarget::Mgm, mgm_request);
        sim_testbench
            .send_request(request)
            .expect("sending MGM request failed");
        sim_testbench.handle_sim_requests();
        sim_testbench.step();
        let mut sim_reply_res = sim_testbench.try_receive_next_reply();
        assert!(sim_reply_res.is_some());
        let mut sim_reply = sim_reply_res.unwrap();
        assert_eq!(sim_reply.target(), SimTarget::Mgm);
        let first_reply: MgmReply = serde_json::from_str(sim_reply.reply())
            .expect("failed to deserialize MGM sensor values");
        let mgm_request = MgmRequest::RequestSensorData;
        sim_testbench.step_by(Duration::from_millis(50));

        request = SimRequest::new(SimTarget::Mgm, mgm_request);
        sim_testbench
            .send_request(request)
            .expect("sending MGM request failed");
        sim_testbench.handle_sim_requests();
        sim_testbench.step();
        sim_reply_res = sim_testbench.try_receive_next_reply();
        assert!(sim_reply_res.is_some());
        sim_reply = sim_reply_res.unwrap();

        let second_reply: MgmReply = serde_json::from_str(sim_reply.reply())
            .expect("failed to deserialize MGM sensor values");
        // Check that the values are changing.
        assert!(first_reply != second_reply);
    }

    #[test]
    fn test_basic_mgt_request_is_off() {
        let mut sim_testbench = SimTestbench::new();
        let mgt_request = MgtRequest::RequestHk;
        let request = SimRequest::new(SimTarget::Mgt, mgt_request);
        sim_testbench
            .send_request(request)
            .expect("sending MGM request failed");
        sim_testbench.handle_sim_requests();
        sim_testbench.step();
        let sim_reply_res = sim_testbench.try_receive_next_reply();
        assert!(sim_reply_res.is_none());
    }

    #[test]
    fn test_basic_mgt_request_is_on() {
        let mut sim_testbench = SimTestbench::new();
        switch_device_on(&mut sim_testbench, PcduSwitch::Mgt);
        let mgt_request = MgtRequest::RequestHk;
        let request = SimRequest::new(SimTarget::Mgt, mgt_request);
        sim_testbench
            .send_request(request)
            .expect("sending MGM request failed");
        sim_testbench.handle_sim_requests();
        sim_testbench.step();
        let sim_reply_res = sim_testbench.try_receive_next_reply();
        assert!(sim_reply_res.is_some());
        let sim_reply = sim_reply_res.unwrap();
        let mgt_reply: MgtReply = serde_json::from_str(sim_reply.reply())
            .expect("failed to deserialize MGM sensor values");
        match mgt_reply {
            MgtReply::Hk(hk) => {
                assert_eq!(hk.dipole, MgtDipole::default());
                assert!(!hk.torquing);
            }
            _ => panic!("unexpected reply"),
        }
    }

    fn check_mgt_hk(sim_testbench: &mut SimTestbench, expected_hk_set: MgtHkSet) {
        let mgt_request = MgtRequest::RequestHk;
        let request = SimRequest::new(SimTarget::Mgt, mgt_request);
        sim_testbench
            .send_request(request)
            .expect("sending MGM request failed");
        sim_testbench.handle_sim_requests();
        sim_testbench.step();
        let sim_reply_res = sim_testbench.try_receive_next_reply();
        assert!(sim_reply_res.is_some());
        let sim_reply = sim_reply_res.unwrap();
        let mgt_reply: MgtReply = serde_json::from_str(sim_reply.reply())
            .expect("failed to deserialize MGM sensor values");
        match mgt_reply {
            MgtReply::Hk(hk) => {
                assert_eq!(hk, expected_hk_set);
            }
            _ => panic!("unexpected reply"),
        }
    }

    #[test]
    fn test_basic_mgt_request_is_on_and_torquing() {
        let mut sim_testbench = SimTestbench::new();
        switch_device_on(&mut sim_testbench, PcduSwitch::Mgt);
        let commanded_dipole = MgtDipole {
            x: -200,
            y: 200,
            z: 1000,
        };
        let mgt_request = MgtRequest::ApplyTorque {
            duration: Duration::from_millis(100),
            dipole: commanded_dipole,
        };
        let request = SimRequest::new(SimTarget::Mgt, mgt_request);
        sim_testbench
            .send_request(request)
            .expect("sending MGM request failed");
        sim_testbench.handle_sim_requests();
        sim_testbench.step_by(Duration::from_millis(5));

        check_mgt_hk(
            &mut sim_testbench,
            MgtHkSet {
                dipole: commanded_dipole,
                torquing: true,
            },
        );
        sim_testbench.step_by(Duration::from_millis(100));
        check_mgt_hk(
            &mut sim_testbench,
            MgtHkSet {
                dipole: MgtDipole::default(),
                torquing: false,
            },
        );
    }
}
