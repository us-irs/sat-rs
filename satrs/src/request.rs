use core::fmt;
#[cfg(feature = "std")]
use std::error::Error;

use spacepackets::{
    ecss::{tc::IsPusTelecommand, PusPacket},
    ByteConversionError, CcsdsPacket,
};

use crate::TargetId;

pub type Apid = u16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetIdCreationError {
    ByteConversion(ByteConversionError),
    NotEnoughAppData(usize),
}

impl From<ByteConversionError> for TargetIdCreationError {
    fn from(e: ByteConversionError) -> Self {
        Self::ByteConversion(e)
    }
}

impl fmt::Display for TargetIdCreationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::ByteConversion(e) => write!(f, "target ID creation: {}", e),
            Self::NotEnoughAppData(len) => {
                write!(f, "not enough app data to generate target ID: {}", len)
            }
        }
    }
}

#[cfg(feature = "std")]
impl Error for TargetIdCreationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let Self::ByteConversion(e) = self {
            return Some(e);
        }
        None
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct TargetAndApidId {
    pub apid: Apid,
    pub target: u32,
}

impl TargetAndApidId {
    pub fn new(apid: Apid, target: u32) -> Self {
        Self { apid, target }
    }

    pub fn apid(&self) -> Apid {
        self.apid
    }

    pub fn target(&self) -> u32 {
        self.target
    }

    pub fn raw(&self) -> TargetId {
        ((self.apid as u64) << 32) | (self.target as u64)
    }

    pub fn target_id(&self) -> TargetId {
        self.raw()
    }

    pub fn from_pus_tc(
        tc: &(impl CcsdsPacket + PusPacket + IsPusTelecommand),
    ) -> Result<Self, TargetIdCreationError> {
        if tc.user_data().len() < 4 {
            return Err(ByteConversionError::FromSliceTooSmall {
                found: tc.user_data().len(),
                expected: 8,
            }
            .into());
        }
        Ok(Self {
            apid: tc.apid(),
            target: u32::from_be_bytes(tc.user_data()[0..4].try_into().unwrap()),
        })
    }
}

impl From<u64> for TargetAndApidId {
    fn from(raw: u64) -> Self {
        Self {
            apid: (raw >> 32) as u16,
            target: raw as u32,
        }
    }
}

impl From<TargetAndApidId> for u64 {
    fn from(target_and_apid_id: TargetAndApidId) -> Self {
        target_and_apid_id.raw()
    }
}

impl fmt::Display for TargetAndApidId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}, {}", self.apid, self.target)
    }
}
