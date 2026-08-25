#[derive(
    serde::Serialize,
    serde::Deserialize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    num_enum::IntoPrimitive,
    num_enum::TryFromPrimitive,
)]
#[repr(u32)]
pub enum Mode {
    Off,
    Normal,
}

pub mod request {
    use super::*;

    #[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub enum ModeRequest {
        SetMode(Mode),
        ReadMode,
    }
}

pub mod response {
    use super::*;

    #[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub enum ModeReport {
        /// Mode of the assembly.
        Mode(super::Mode),
        /// Children are in wrong mode after commanding.
        WrongMode([Option<Mode>; 2]),
    }
}
