pub mod request {
    use crate::HkRequestType;

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
    }
}

#[derive(Default, Debug, Copy, Clone, serde::Serialize, serde::Deserialize)]
pub struct MgmData {
    pub valid: bool,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub mod response {
    use crate::{Message, mgm::MgmData};

    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
    pub enum HkResponse {
        MgmData(MgmData),
    }

    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
    pub enum Response {
        Ok,
        Hk(HkResponse),
    }

    impl Message for Response {
        fn message_type(&self) -> crate::MessageType {
            match self {
                Response::Ok => crate::MessageType::Verification,
                Response::Hk(_hk_response) => crate::MessageType::Hk,
            }
        }
    }
}
