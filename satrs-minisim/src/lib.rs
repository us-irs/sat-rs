use serde::{de::DeserializeOwned, Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimTarget {
    SimCtrl,
    Mgm,
    Mgt,
    Pcdu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimMessage {
    pub target: SimTarget,
    pub payload: String,
}

/// A generic simulation request type. Right now, the payload data is expected to be
/// JSON, which might be changed in the future.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimRequest {
    inner: SimMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimMessageType {
    Request,
    Reply,
}

/// Generic trait implemented by simulation request or reply payloads. It ties the request or
/// reply to a specific target and provides an API which does boilerplate tasks like checking the
/// validity of the target.
pub trait SerializableSimMsgPayload<P: SimMessageProvider>:
    Serialize + DeserializeOwned + Sized
{
    const TARGET: SimTarget;

    fn from_sim_message(sim_message: &P) -> Result<Self, SimMessageError<P>> {
        if sim_message.target() == Self::TARGET {
            return Ok(serde_json::from_str(sim_message.payload())?);
        }
        Err(SimMessageError::TargetRequestMissmatch(sim_message.clone()))
    }
}

pub trait SimMessageProvider: Serialize + DeserializeOwned + Clone + Sized {
    fn msg_type(&self) -> SimMessageType;
    fn target(&self) -> SimTarget;
    fn payload(&self) -> &String;
    fn from_raw_data(data: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(data)
    }
}

impl SimRequest {
    pub fn new<T: SerializableSimMsgPayload<SimRequest>>(serializable_request: T) -> Self {
        Self {
            inner: SimMessage {
                target: T::TARGET,
                payload: serde_json::to_string(&serializable_request).unwrap(),
            },
        }
    }
}

impl SimMessageProvider for SimRequest {
    fn target(&self) -> SimTarget {
        self.inner.target
    }
    fn payload(&self) -> &String {
        &self.inner.payload
    }

    fn msg_type(&self) -> SimMessageType {
        SimMessageType::Request
    }
}

/// A generic simulation reply type. Right now, the payload data is expected to be
/// JSON, which might be changed inthe future.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimReply {
    inner: SimMessage,
}

impl SimReply {
    pub fn new<T: SerializableSimMsgPayload<SimReply>>(serializable_reply: T) -> Self {
        Self {
            inner: SimMessage {
                target: T::TARGET,
                payload: serde_json::to_string(&serializable_reply).unwrap(),
            },
        }
    }
}

impl SimMessageProvider for SimReply {
    fn target(&self) -> SimTarget {
        self.inner.target
    }
    fn payload(&self) -> &String {
        &self.inner.payload
    }
    fn msg_type(&self) -> SimMessageType {
        SimMessageType::Reply
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimCtrlRequest {
    Ping,
}

impl SerializableSimMsgPayload<SimRequest> for SimCtrlRequest {
    const TARGET: SimTarget = SimTarget::SimCtrl;
}

pub type SimReplyError = SimMessageError<SimReply>;
pub type SimRequestError = SimMessageError<SimRequest>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimMessageError<P> {
    SerdeJson(String),
    TargetRequestMissmatch(P),
}

impl<P> From<serde_json::Error> for SimMessageError<P> {
    fn from(error: serde_json::Error) -> SimMessageError<P> {
        SimMessageError::SerdeJson(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimCtrlReply {
    Pong,
    InvalidRequest(SimRequestError),
}

impl SerializableSimMsgPayload<SimReply> for SimCtrlReply {
    const TARGET: SimTarget = SimTarget::SimCtrl;
}

impl From<SimRequestError> for SimCtrlReply {
    fn from(error: SimRequestError) -> Self {
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

    impl SerializableSimMsgPayload<SimRequest> for PcduRequest {
        const TARGET: SimTarget = SimTarget::Pcdu;
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum PcduReply {
        SwitchInfo(SwitchMap),
    }

    impl SerializableSimMsgPayload<SimReply> for PcduReply {
        const TARGET: SimTarget = SimTarget::Pcdu;
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

    impl SerializableSimMsgPayload<SimRequest> for MgmRequest {
        const TARGET: SimTarget = SimTarget::Mgm;
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

    impl SerializableSimMsgPayload<SimReply> for MgmReply {
        const TARGET: SimTarget = SimTarget::Mgm;
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

    impl SerializableSimMsgPayload<SimRequest> for MgtRequest {
        const TARGET: SimTarget = SimTarget::Mgt;
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

    impl SerializableSimMsgPayload<SimReply> for MgtReply {
        const TARGET: SimTarget = SimTarget::Mgm;
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

#[cfg(test)]
pub mod tests {
    use super::*;

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DummyRequest {
        Ping,
    }

    impl SerializableSimMsgPayload<SimRequest> for DummyRequest {
        const TARGET: SimTarget = SimTarget::SimCtrl;
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DummyReply {
        Pong,
    }

    impl SerializableSimMsgPayload<SimReply> for DummyReply {
        const TARGET: SimTarget = SimTarget::SimCtrl;
    }

    #[test]
    fn test_basic_request() {
        let sim_request = SimRequest::new(DummyRequest::Ping);
        assert_eq!(sim_request.target(), SimTarget::SimCtrl);
        assert_eq!(sim_request.msg_type(), SimMessageType::Request);
        let dummy_request =
            DummyRequest::from_sim_message(&sim_request).expect("deserialization failed");
        assert_eq!(dummy_request, DummyRequest::Ping);
    }

    #[test]
    fn test_basic_reply() {
        let sim_reply = SimReply::new(DummyReply::Pong);
        assert_eq!(sim_reply.target(), SimTarget::SimCtrl);
        assert_eq!(sim_reply.msg_type(), SimMessageType::Reply);
        let dummy_request =
            DummyReply::from_sim_message(&sim_reply).expect("deserialization failed");
        assert_eq!(dummy_request, DummyReply::Pong);
    }
}
