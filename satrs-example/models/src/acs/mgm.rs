pub mod request {
    use crate::{DeviceMode, HkRequestType, Message};

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ModeRequest {
        SetMode(DeviceMode),
        ReadMode,
    }

    #[derive(Debug, PartialEq, Eq, Clone, Copy, serde::Serialize, serde::Deserialize)]
    pub enum HkId {
        Sensor,
    }

    #[derive(Debug, PartialEq, Eq, Clone, Copy, serde::Serialize, serde::Deserialize)]
    pub struct HkRequest {
        pub id: HkId,
        pub req_type: HkRequestType,
    }

    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
    pub enum Request {
        Ping,
        Hk(HkRequest),
        Mode(ModeRequest),
    }

    impl Request {
        fn message_type(&self) -> crate::MessageType {
            match self {
                Request::Ping => crate::MessageType::Verification,
                Request::Hk(_hk_request) => crate::MessageType::Hk,
                Request::Mode(_mode) => crate::MessageType::Mode,
            }
        }
    }
    impl Message for Request {
        fn message_type(&self) -> crate::MessageType {
            self.message_type()
        }
    }
}

#[derive(Default, Debug, Copy, Clone, serde::Serialize, serde::Deserialize)]
pub struct SensorData {
    pub valid: bool,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub mod response {
    use crate::{DeviceMode, Message, acs::mgm::SensorData};

    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
    pub enum HkResponse {
        MgmData(SensorData),
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
    pub enum ModeResponse {
        /// New mode has been set.
        Mode(DeviceMode),
        /// Setting a mode timed out.
        SetModeTimeout,
    }

    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
    pub enum Response {
        Ok,
        Hk(HkResponse),
        Mode(ModeResponse),
    }

    impl Response {
        fn message_type(&self) -> crate::MessageType {
            match self {
                Response::Ok => crate::MessageType::Verification,
                Response::Hk(_hk_response) => crate::MessageType::Hk,
                Response::Mode(_mode_failure) => crate::MessageType::Mode,
            }
        }
    }

    impl Message for Response {
        fn message_type(&self) -> crate::MessageType {
            self.message_type()
        }
    }
}
