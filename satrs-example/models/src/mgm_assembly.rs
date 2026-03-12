use core::str::FromStr;

use crate::DeviceMode;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssemblyMode {
    /// The assembly mode ressembles the modes of the devices it controls. It also tries to keep
    /// the children in the correct mode by re-commanding them into the correct mode.
    Device(DeviceMode),
    /// Mode keeping disabled.
    NoModeKeeping,
}

impl FromStr for AssemblyMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "off" => Ok(AssemblyMode::Device(DeviceMode::Off)),
            "on" => Ok(AssemblyMode::Device(DeviceMode::On)),
            "normal" => Ok(AssemblyMode::Device(DeviceMode::Normal)),
            "no_mode_keeping" => Ok(AssemblyMode::NoModeKeeping),
            _ => Err(()),
        }
    }
}

pub mod request {
    use crate::{HkRequestType, Message, mgm_assembly::AssemblyMode};

    #[derive(Debug, PartialEq, Eq, Clone, Copy, serde::Serialize, serde::Deserialize)]
    pub enum HkId {
        Sensor,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub enum ModeRequest {
        SetMode(AssemblyMode),
        ReadMode,
    }

    #[derive(Debug, PartialEq, Eq, Clone, Copy, serde::Serialize, serde::Deserialize)]
    pub struct HkRequest {
        pub id: HkId,
        pub req_type: HkRequestType,
    }

    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
    pub enum Request {
        Ping,
        Mode(ModeRequest),
    }

    impl Request {
        fn message_type(&self) -> crate::MessageType {
            match self {
                Request::Ping => crate::MessageType::Verification,
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

pub mod response {
    use crate::{DeviceMode, Message};

    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ModeCommandFailure {
        Timeout,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub enum ModeReport {
        /// Mode of the assembly.
        Mode(super::AssemblyMode),
        /// Timeout failure setting the children modes.
        SetModeTimeout([Option<DeviceMode>; 2]),
        /// Children are in wrong mode after commanding.
        WrongMode([Option<DeviceMode>; 2]),
        /// An assembly tried modekeeping but can not keep its mode.
        CanNotKeepMode([Option<DeviceMode>; 2]),
    }

    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Response {
        Ok,
        Mode(ModeReport),
    }

    impl Response {
        fn message_type(&self) -> crate::MessageType {
            match self {
                Response::Ok => crate::MessageType::Verification,
                Response::Mode(_mode_report) => crate::MessageType::Mode,
            }
        }
    }

    impl Message for Response {
        fn message_type(&self) -> crate::MessageType {
            self.message_type()
        }
    }
}
