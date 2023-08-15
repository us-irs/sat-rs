use delegate::delegate;
use derive_new::new;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use satrs_core::events::{EventU32TypedSev, SeverityInfo};
use satrs_core::objects::ObjectId;
use satrs_core::spacepackets::ecss::tc::{GenericPusTcSecondaryHeader, IsPusTelecommand, PusTc};
use satrs_core::spacepackets::ecss::PusPacket;
use satrs_core::spacepackets::{ByteConversionError, CcsdsPacket, SizeMissmatch};
use satrs_core::tmtc::TargetId;
use std::fmt;
use std::net::Ipv4Addr;
use thiserror::Error;

use satrs_mib::res_code::{ResultU16, ResultU16Info};
use satrs_mib::resultcode;

//pub mod can;
//mod can_ids;
mod logger;

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
            return Err(ByteConversionError::FromSliceTooSmall(SizeMissmatch {
                found: tc.user_data().len(),
                expected: 8,
            })
            .into());
        }
        let target_id = u32::from_be_bytes(tc.user_data()[0..4].try_into().unwrap());
        Ok(Self {
            apid: tc.apid(),
            target: u32::from_be_bytes(tc.user_data()[0..4].try_into().unwrap()),
        })
    }
}

// #[derive(Copy, Clone, PartialEq, Eq, Debug, Hash, new)]
// #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
// pub struct TargetIdWithUniqueId {
//     target_id: TargetIdWithApid,
//     unique_id: u32,
// }
//
// impl TargetIdWithUniqueId {
//     delegate! {
//        to self.target_id {
//             pub fn apid(&self) -> Apid;
//             pub fn target_id(&self) -> TargetId;
//             }
//         }
//     pub fn unique_id(&self) -> u32 {
//         self.unique_id
//     }
//
//     pub fn write_target_id_and_unique_id_as_pus_header(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError> {
//        if buf.len() < 8 {
//            return Err(ByteConversionError::ToSliceTooSmall(SizeMissmatch {
//                found: buf.len(),
//                expected: 8,
//            }));
//        }
//         buf[0..4].copy_from_slice(&self.target_id.target_id().to_be_bytes());
//         buf[4..8].copy_from_slice(&self.unique_id.to_be_bytes());
//         Ok(8)
//     }
// }
//
pub const PUS_APID: u16 = 0x02;

#[derive(Copy, Clone, PartialEq, Eq, Debug, TryFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum CustomPusServiceId {
    Mode = 200,
    Health = 201,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum RequestTargetId {
    AcsSubsystem = 1,
}

pub const AOCS_APID: u16 = 1;

pub const ACS_OBJECT_ID: ObjectId = ObjectId {
    id: RequestTargetId::AcsSubsystem as u32,
    name: "ACS_SUBSYSTEM",
};

#[derive(Debug)]
pub enum GroupId {
    Tmtc = 0,
    Hk = 1,
}

pub const OBSW_SERVER_ADDR: Ipv4Addr = Ipv4Addr::UNSPECIFIED;
pub const SERVER_PORT: u16 = 7301;

pub const TEST_EVENT: EventU32TypedSev<SeverityInfo> =
    EventU32TypedSev::<SeverityInfo>::const_new(0, 0);

pub mod tmtc_err {
    use super::*;

    #[resultcode]
    pub const INVALID_PUS_SERVICE: ResultU16 = ResultU16::const_new(GroupId::Tmtc as u8, 0);
    #[resultcode]
    pub const INVALID_PUS_SUBSERVICE: ResultU16 = ResultU16::const_new(GroupId::Tmtc as u8, 1);
    #[resultcode]
    pub const PUS_SERVICE_NOT_IMPLEMENTED: ResultU16 = ResultU16::const_new(GroupId::Tmtc as u8, 2);
    #[resultcode]
    pub const UNKNOWN_TARGET_ID: ResultU16 = ResultU16::const_new(GroupId::Tmtc as u8, 3);

    #[resultcode(
        info = "Not enough data inside the TC application data field. Optionally includes: \
          8 bytes of failure data containing 2 failure parameters, \
          P1 (u32 big endian): Expected data length, P2: Found data length"
    )]
    pub const NOT_ENOUGH_APP_DATA: ResultU16 = ResultU16::const_new(GroupId::Tmtc as u8, 2);

    pub const TMTC_RESULTS: &[ResultU16Info] = &[
        INVALID_PUS_SERVICE_EXT,
        INVALID_PUS_SUBSERVICE_EXT,
        NOT_ENOUGH_APP_DATA_EXT,
    ];
}

pub mod hk_err {
    use super::*;

    #[resultcode]
    pub const TARGET_ID_MISSING: ResultU16 = ResultU16::const_new(GroupId::Hk as u8, 0);
    #[resultcode]
    pub const UNIQUE_ID_MISSING: ResultU16 = ResultU16::const_new(GroupId::Hk as u8, 1);
    #[resultcode]
    pub const UNKNOWN_TARGET_ID: ResultU16 = ResultU16::const_new(GroupId::Hk as u8, 2);
    #[resultcode]
    pub const COLLECTION_INTERVAL_MISSING: ResultU16 = ResultU16::const_new(GroupId::Hk as u8, 3);
}

#[allow(clippy::enum_variant_names)]
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum TmSenderId {
    PusVerification = 0,
    PusTest = 1,
    PusEvent = 2,
    PusHk = 3,
    PusAction = 4,
    PusSched = 5,
    AllEvents = 6,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum TcReceiverId {
    PusTest = 1,
    PusEvent = 2,
    PusHk = 3,
    PusAction = 4,
    PusSched = 5,
}
