use crate::tmtc::TargetId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModePair {
    mode: u32,
    submode: u16,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModeCommand {
    address: TargetId,
    mode: ModePair,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ModeRequest {
    SetMode(ModeCommand),
    ReadMode(TargetId),
    AnnounceMode(TargetId),
    AnnounceModeRecursive(TargetId),
}
