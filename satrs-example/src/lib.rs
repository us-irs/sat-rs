use std::net::Ipv4Addr;
use satrs_core::events::{EventU32, EventU32TypedSev, Severity, SeverityInfo};

use satrs_mib::res_code::{ResultU16, ResultU16Info};
use satrs_mib::resultcode;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum RequestTargetId {
    AcsSubsystem = 1,
}

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

    #[resultcode(info = "Not enough data inside the TC application data field")]
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


