use crate::tmtc::TargetId;
use core::mem::size_of;
use serde::{Deserialize, Serialize};
use spacepackets::{ByteConversionError, SizeMissmatch};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModeAndSubmode {
    mode: u32,
    submode: u16,
}

impl ModeAndSubmode {
    pub const fn new_mode_only(mode: u32) -> Self {
        Self { mode, submode: 0 }
    }

    pub const fn new(mode: u32, submode: u16) -> Self {
        Self { mode, submode }
    }

    pub fn raw_len() -> usize {
        size_of::<u32>() + size_of::<u16>()
    }

    pub fn from_be_bytes(buf: &[u8]) -> Result<Self, ByteConversionError> {
        if buf.len() < 6 {
            return Err(ByteConversionError::FromSliceTooSmall(SizeMissmatch {
                expected: 6,
                found: buf.len(),
            }));
        }
        Ok(Self {
            mode: u32::from_be_bytes(buf[0..4].try_into().unwrap()),
            submode: u16::from_be_bytes(buf[4..6].try_into().unwrap()),
        })
    }

    pub fn mode(&self) -> u32 {
        self.mode
    }

    pub fn submode(&self) -> u16 {
        self.submode
    }
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModeCommand {
    pub address: TargetId,
    pub mode_submode: ModeAndSubmode,
}

impl ModeCommand {
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
pub struct TargetedModeRequest {
    target_id: TargetId,
    mode_request: ModeRequest,
}
