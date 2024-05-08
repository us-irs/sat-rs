use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    sync::mpsc,
    time::Duration,
};

use satrs::pus::HandlingStatus;
use satrs_minisim::{
    udp::SIM_CTRL_PORT, SerializableSimMsgPayload, SimComponent, SimMessageProvider, SimReply,
    SimRequest,
};
use satrs_minisim::{SimCtrlReply, SimCtrlRequest};

struct SimReplyMap(pub HashMap<SimComponent, mpsc::Sender<SimReply>>);

pub fn create_sim_client(sim_request_rx: mpsc::Receiver<SimRequest>) -> Option<SimClientUdp> {
    match SimClientUdp::new(
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, SIM_CTRL_PORT)),
        sim_request_rx,
    ) {
        Ok(sim_client) => {
            log::info!("simulator client connection success");
            return Some(sim_client);
        }
        Err(e) => {
            log::warn!("sim client creation error: {}", e);
        }
    }
    None
}

#[derive(thiserror::Error, Debug)]
pub enum SimClientCreationResult {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("timeout when trying to connect to sim UDP server")]
    Timeout,
    #[error("invalid ping reply when trying connection to UDP sim server")]
    InvalidReplyJsonError(#[from] serde_json::Error),
    #[error("invalid sim reply, not pong reply as expected: {0:?}")]
    ReplyIsNotPong(SimReply),
}

pub struct SimClientUdp {
    udp_client: UdpSocket,
    simulator_addr: SocketAddr,
    sim_request_rx: mpsc::Receiver<SimRequest>,
    reply_map: SimReplyMap,
    reply_buf: [u8; 4096],
}

impl SimClientUdp {
    pub fn new(
        simulator_addr: SocketAddr,
        sim_request_rx: mpsc::Receiver<SimRequest>,
    ) -> Result<Self, SimClientCreationResult> {
        let mut reply_buf: [u8; 4096] = [0; 4096];
        let udp_client = UdpSocket::bind("127.0.0.1:0")?;
        udp_client.set_read_timeout(Some(Duration::from_millis(100)))?;
        let sim_req = SimRequest::new_with_epoch_time(SimCtrlRequest::Ping);
        let sim_req_json = serde_json::to_string(&sim_req).expect("failed to serialize SimRequest");
        udp_client.send_to(sim_req_json.as_bytes(), simulator_addr)?;
        match udp_client.recv(&mut reply_buf) {
            Ok(reply_len) => {
                let sim_reply: SimReply = serde_json::from_slice(&reply_buf[0..reply_len])?;
                if sim_reply.component() != SimComponent::SimCtrl {
                    return Err(SimClientCreationResult::ReplyIsNotPong(sim_reply));
                }
                udp_client.set_read_timeout(None)?;
                let sim_ctrl_reply =
                    SimCtrlReply::from_sim_message(&sim_reply).expect("invalid SIM reply");
                match sim_ctrl_reply {
                    SimCtrlReply::Pong => Ok(Self {
                        udp_client,
                        simulator_addr,
                        sim_request_rx,
                        reply_map: SimReplyMap(HashMap::new()),
                        reply_buf,
                    }),
                    SimCtrlReply::InvalidRequest(_) => {
                        panic!("received invalid request reply from UDP sim server")
                    }
                }
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock
                {
                    Err(SimClientCreationResult::Timeout)
                } else {
                    Err(SimClientCreationResult::Io(e))
                }
            }
        }
    }

    pub fn operation(&mut self) -> HandlingStatus {
        let mut no_sim_requests_handled = true;
        let mut no_data_from_udp_server_received = true;
        loop {
            match self.sim_request_rx.try_recv() {
                Ok(request) => {
                    let request_json =
                        serde_json::to_string(&request).expect("failed to serialize SimRequest");
                    if let Err(e) = self
                        .udp_client
                        .send_to(request_json.as_bytes(), self.simulator_addr)
                    {
                        log::error!("error sending data to UDP SIM server: {}", e);
                        break;
                    } else {
                        no_sim_requests_handled = false;
                    }
                }
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => {
                        break;
                    }
                    mpsc::TryRecvError::Disconnected => {
                        log::warn!("SIM request sender disconnected");
                        break;
                    }
                },
            }
        }
        loop {
            match self.udp_client.recv(&mut self.reply_buf) {
                Ok(recvd_bytes) => {
                    no_data_from_udp_server_received = false;
                    let sim_reply_result: serde_json::Result<SimReply> =
                        serde_json::from_slice(&self.reply_buf[0..recvd_bytes]);
                    match sim_reply_result {
                        Ok(sim_reply) => {
                            if let Some(sender) = self.reply_map.0.get(&sim_reply.component()) {
                                sender.send(sim_reply).expect("failed to send SIM reply");
                            } else {
                                log::warn!(
                                    "no recipient for SIM reply from component {:?}",
                                    sim_reply.component()
                                );
                            }
                        }
                        Err(e) => {
                            log::warn!("failed to deserialize SIM reply: {}", e);
                        }
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut
                    {
                        break;
                    }
                    log::error!("error receiving data from UDP SIM server: {}", e);
                    break;
                }
            }
        }
        if no_sim_requests_handled && no_data_from_udp_server_received {
            return HandlingStatus::Empty;
        }
        HandlingStatus::HandledOne
    }

    pub fn add_reply_recipient(
        &mut self,
        component: SimComponent,
        reply_sender: mpsc::Sender<SimReply>,
    ) {
        self.reply_map.0.insert(component, reply_sender);
    }
}

#[cfg(test)]
pub mod tests {
    // TODO: Write some basic tests which verify that the ping/pong handling/check for the
    // constructor works as expected.
    fn test_basic() {}
}
