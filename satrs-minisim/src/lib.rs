use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimTarget {
    SimCtrl,
    Mgm,
    Mgt,
    Pcdu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimRequest {
    target: SimTarget,
    request: String,
}

impl SimRequest {
    pub fn new<T: Serialize>(device: SimTarget, request: T) -> Self {
        Self {
            target: device,
            request: serde_json::to_string(&request).unwrap(),
        }
    }

    pub fn target(&self) -> SimTarget {
        self.target
    }

    pub fn request(&self) -> &String {
        &self.request
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimReply {
    target: SimTarget,
    reply: String,
}

impl SimReply {
    pub fn new<T: Serialize>(device: SimTarget, reply: T) -> Self {
        Self {
            target: device,
            reply: serde_json::to_string(&reply).unwrap(),
        }
    }

    pub fn target(&self) -> SimTarget {
        self.target
    }

    pub fn reply(&self) -> &String {
        &self.reply
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimCtrlRequest {
    Ping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestError {
    TargetRequestMissmatch(SimRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimCtrlReply {
    Pong,
    InvalidRequest(RequestError),
}

impl From<RequestError> for SimCtrlReply {
    fn from(error: RequestError) -> Self {
        SimCtrlReply::InvalidRequest(error)
    }
}

pub mod eps {
    use super::*;
    use std::collections::HashMap;

    use satrs::power::SwitchStateBinary;

    pub type SwitchMap = HashMap<PcduSwitch, SwitchStateBinary>;

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
    pub enum PcduSwitch {
        Mgm = 0,
        Mgt = 1,
    }

    #[derive(Debug, Copy, Clone, Serialize, Deserialize)]
    pub enum PcduRequest {
        SwitchDevice {
            switch: PcduSwitch,
            state: SwitchStateBinary,
        },
        RequestSwitchInfo,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum PcduReply {
        SwitchInfo(SwitchMap),
    }
}

pub mod acs {
    use std::time::Duration;

    use satrs::power::SwitchStateBinary;

    use super::*;

    #[derive(Debug, Copy, Clone, Serialize, Deserialize)]
    pub enum MgmRequest {
        RequestSensorData,
    }

    // Normally, small magnetometers generate their output as a signed 16 bit raw format or something
    // similar which needs to be converted to a signed float value with physical units. We will
    // simplify this now and generate the signed float values directly.
    #[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
    pub struct MgmSensorValues {
        pub x: f32,
        pub y: f32,
        pub z: f32,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
    pub struct MgmReply {
        pub switch_state: SwitchStateBinary,
        pub sensor_values: MgmSensorValues,
    }

    pub const MGT_GEN_MAGNETIC_FIELD: MgmSensorValues = MgmSensorValues {
        x: 0.03,
        y: -0.03,
        z: 0.03,
    };

    // Simple model using i16 values.
    #[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct MgtDipole {
        pub x: i16,
        pub y: i16,
        pub z: i16,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
    pub enum MgtRequestType {
        ApplyTorque,
    }

    #[derive(Debug, Copy, Clone, Serialize, Deserialize)]
    pub enum MgtRequest {
        ApplyTorque {
            duration: Duration,
            dipole: MgtDipole,
        },
        RequestHk,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct MgtHkSet {
        pub dipole: MgtDipole,
        pub torquing: bool,
    }

    #[derive(Debug, Copy, Clone, Serialize, Deserialize)]
    pub enum MgtReply {
        Ack(MgtRequestType),
        Nak(MgtRequestType),
        Hk(MgtHkSet),
    }
}

pub mod udp {
    use std::{
        net::{SocketAddr, UdpSocket},
        time::Duration,
    };

    use thiserror::Error;

    use crate::{SimReply, SimRequest};

    #[derive(Error, Debug)]
    pub enum ReceptionError {
        #[error("IO error: {0}")]
        Io(#[from] std::io::Error),
        #[error("Serde JSON error: {0}")]
        SerdeJson(#[from] serde_json::Error),
    }

    pub struct SimUdpClient {
        socket: UdpSocket,
        pub reply_buf: [u8; 4096],
    }

    impl SimUdpClient {
        pub fn new(
            server_addr: &SocketAddr,
            non_blocking: bool,
            read_timeot_ms: Option<u64>,
        ) -> std::io::Result<Self> {
            let socket = UdpSocket::bind("127.0.0.1:0")?;
            socket.set_nonblocking(non_blocking)?;
            socket
                .connect(server_addr)
                .expect("could not connect to server addr");
            if let Some(read_timeout) = read_timeot_ms {
                // Set a read timeout so the test does not hang on failures.
                socket.set_read_timeout(Some(Duration::from_millis(read_timeout)))?;
            }
            Ok(Self {
                socket,
                reply_buf: [0; 4096],
            })
        }

        pub fn set_nonblocking(&self, non_blocking: bool) -> std::io::Result<()> {
            self.socket.set_nonblocking(non_blocking)
        }

        pub fn set_read_timeout(&self, read_timeout_ms: u64) -> std::io::Result<()> {
            self.socket
                .set_read_timeout(Some(Duration::from_millis(read_timeout_ms)))
        }

        pub fn send_request(&self, sim_request: &SimRequest) -> std::io::Result<usize> {
            self.socket.send(
                &serde_json::to_vec(sim_request).expect("conversion of request to vector failed"),
            )
        }

        pub fn recv_raw(&mut self) -> std::io::Result<usize> {
            self.socket.recv(&mut self.reply_buf)
        }

        pub fn recv_sim_reply(&mut self) -> Result<SimReply, ReceptionError> {
            let read_len = self.recv_raw()?;
            Ok(serde_json::from_slice(&self.reply_buf[0..read_len])?)
        }
    }
}
