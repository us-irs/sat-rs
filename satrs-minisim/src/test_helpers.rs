use delegate::delegate;
use std::{sync::mpsc, time::Duration};

use asynchronix::time::MonotonicTime;
use satrs_minisim::{SimReply, SimRequest};

use crate::{controller::SimController, create_sim_controller, ThreadingModel};

pub struct SimTestbench {
    pub sim_controller: SimController,
    pub reply_receiver: mpsc::Receiver<SimReply>,
    pub request_sender: mpsc::Sender<SimRequest>,
}

impl SimTestbench {
    pub fn new() -> Self {
        let (request_sender, request_receiver) = mpsc::channel();
        let (reply_sender, reply_receiver) = mpsc::channel();
        let t0 = MonotonicTime::EPOCH;
        let sim_ctrl =
            create_sim_controller(ThreadingModel::Single, t0, reply_sender, request_receiver);

        Self {
            sim_controller: sim_ctrl,
            reply_receiver,
            request_sender,
        }
    }
    pub fn handle_sim_requests_time_agnostic(&mut self) {
        self.handle_sim_requests(MonotonicTime::EPOCH);
    }

    delegate! {
        to self.sim_controller {
            pub fn handle_sim_requests(&mut self, old_timestamp: MonotonicTime);
        }
        to self.sim_controller.simulation {
            pub fn step(&mut self);
            pub fn step_by(&mut self, duration: Duration);
        }
    }

    pub fn send_request(&self, request: SimRequest) -> Result<(), mpsc::SendError<SimRequest>> {
        self.request_sender.send(request)
    }

    pub fn try_receive_next_reply(&self) -> Option<SimReply> {
        match self.reply_receiver.try_recv() {
            Ok(reply) => Some(reply),
            Err(e) => {
                if e == mpsc::TryRecvError::Empty {
                    None
                } else {
                    panic!("reply_receiver disconnected");
                }
            }
        }
    }
}
