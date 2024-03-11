use acs::{MagnetometerModel, MagnetorquerModel};
use asynchronix::simulation::{Mailbox, SimInit};
use asynchronix::time::{MonotonicTime, SystemClock};
use controller::SimController;
use eps::PcduModel;
use satrs_minisim::{SimReply, SimRequest};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};
use udp::SimUdpServer;

mod acs;
mod controller;
mod eps;
#[cfg(test)]
mod test_helpers;
mod time;
mod udp;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ThreadingModel {
    Default = 0,
    Single = 1,
}

fn create_sim_controller(
    threading_model: ThreadingModel,
    start_time: MonotonicTime,
    reply_sender: mpsc::Sender<SimReply>,
    request_receiver: mpsc::Receiver<SimRequest>,
) -> SimController {
    // Instantiate models and their mailboxes.
    let mgm_model = MagnetometerModel::new(Duration::from_millis(50), reply_sender.clone());

    let mgm_mailbox = Mailbox::new();
    let mgm_addr = mgm_mailbox.address();
    let pcdu_mailbox = Mailbox::new();
    let pcdu_addr = pcdu_mailbox.address();
    let mgt_mailbox = Mailbox::new();
    let mgt_addr = mgt_mailbox.address();

    let mut pcdu_model = PcduModel::new(reply_sender.clone());
    pcdu_model
        .mgm_switch
        .connect(MagnetometerModel::switch_device, &mgm_addr);

    let mut mgt_model = MagnetorquerModel::new(reply_sender.clone());
    // Input connections.
    pcdu_model
        .mgt_switch
        .connect(MagnetorquerModel::switch_device, &mgt_addr);
    // Output connections.
    mgt_model
        .gen_magnetic_field
        .connect(MagnetometerModel::apply_external_magnetic_field, &mgm_addr);

    // Instantiate the simulator
    let sys_clock = SystemClock::from_system_time(start_time, SystemTime::now());
    let sim_init = if threading_model == ThreadingModel::Single {
        SimInit::with_num_threads(1)
    } else {
        SimInit::new()
    };
    let simulation = sim_init
        .add_model(mgm_model, mgm_mailbox)
        .add_model(pcdu_model, pcdu_mailbox)
        .add_model(mgt_model, mgt_mailbox)
        .init(start_time);
    SimController::new(
        sys_clock,
        request_receiver,
        reply_sender,
        simulation,
        mgm_addr,
        pcdu_addr,
        mgt_addr,
    )
}

fn main() {
    let (request_sender, request_receiver) = mpsc::channel();
    let (reply_sender, reply_receiver) = mpsc::channel();
    let t0 = MonotonicTime::EPOCH;
    let mut sim_ctrl =
        create_sim_controller(ThreadingModel::Default, t0, reply_sender, request_receiver);

    // This thread schedules the simulator.
    let sim_thread = thread::spawn(move || {
        sim_ctrl.run(t0, 1);
    });

    let mut udp_server = SimUdpServer::new(0, request_sender, reply_receiver, 200, None)
        .expect("could not create UDP request server");
    // This thread manages the simulator UDP server.
    let udp_tc_thread = thread::spawn(move || {
        udp_server.run();
    });

    sim_thread.join().expect("joining simulation thread failed");
    udp_tc_thread
        .join()
        .expect("joining UDP server thread failed");
}
