use derive_new::new;
use satrs_core::spacepackets::ecss::tc::IsPusTelecommand;
use satrs_core::spacepackets::ecss::PusPacket;
use satrs_core::spacepackets::{ByteConversionError, CcsdsPacket};
use satrs_core::tmtc::TargetId;
use std::fmt;
use thiserror::Error;

pub mod config;

pub type Apid = u16;

#[derive(Debug, Error)]
pub enum TargetIdCreationError {
    #[error("byte conversion")]
    ByteConversion(#[from] ByteConversionError),
    #[error("not enough app data to generate target ID")]
    NotEnoughAppData(usize),
}

// TODO: can these stay pub?
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, new)]
pub struct TargetIdWithApid {
    pub apid: Apid,
    pub target: TargetId,
}

impl fmt::Display for TargetIdWithApid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}, {}", self.apid, self.target)
    }
}

impl TargetIdWithApid {
    pub fn apid(&self) -> Apid {
        self.apid
    }
    pub fn target_id(&self) -> TargetId {
        self.target
    }
}

impl TargetIdWithApid {
    pub fn from_tc(
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
