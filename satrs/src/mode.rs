use core::mem::size_of;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use spacepackets::ByteConversionError;

use crate::TargetId;

pub type Mode = u32;
pub type Submode = u16;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModeAndSubmode {
    mode: Mode,
    submode: Submode,
}

impl ModeAndSubmode {
    pub const fn new_mode_only(mode: Mode) -> Self {
        Self { mode, submode: 0 }
    }

    pub const fn new(mode: Mode, submode: Submode) -> Self {
        Self { mode, submode }
    }

    pub fn raw_len() -> usize {
        size_of::<u32>() + size_of::<u16>()
    }

    pub fn from_be_bytes(buf: &[u8]) -> Result<Self, ByteConversionError> {
        if buf.len() < 6 {
            return Err(ByteConversionError::FromSliceTooSmall {
                expected: 6,
                found: buf.len(),
            });
        }
        Ok(Self {
            mode: Mode::from_be_bytes(buf[0..size_of::<Mode>()].try_into().unwrap()),
            submode: Submode::from_be_bytes(
                buf[size_of::<Mode>()..size_of::<Mode>() + size_of::<Submode>()]
                    .try_into()
                    .unwrap(),
            ),
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn submode(&self) -> Submode {
        self.submode
    }
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TargetedModeCommand {
    pub address: TargetId,
    pub mode_submode: ModeAndSubmode,
}

impl TargetedModeCommand {
    pub const fn new(address: TargetId, mode_submode: ModeAndSubmode) -> Self {
        Self {
            address,
            mode_submode,
        }
    }

    pub fn address(&self) -> TargetId {
        self.address
    }

    pub fn mode_submode(&self) -> ModeAndSubmode {
        self.mode_submode
    }

    pub fn mode(&self) -> u32 {
        self.mode_submode.mode
    }

    pub fn submode(&self) -> u16 {
        self.mode_submode.submode
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ModeRequest {
    SetMode(ModeAndSubmode),
    ReadMode,
    AnnounceMode,
    AnnounceModeRecursive,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ModeReply {
    /// Unrequest mode information. Can be used to notify other components of changed modes.
    ModeInfo(ModeAndSubmode),
    /// Reply to a mode request to confirm the commanded mode was reached.
    ModeReply(ModeAndSubmode),
    CantReachMode(ModeAndSubmode),
    WrongMode {
        expected: ModeAndSubmode,
        reached: ModeAndSubmode,
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TargetedModeRequest {
    target_id: TargetId,
    mode_request: ModeRequest,
}
