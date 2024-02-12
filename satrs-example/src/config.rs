use satrs_core::res_code::ResultU16;
use satrs_mib::res_code::ResultU16Info;
use satrs_mib::resultcode;
use std::net::Ipv4Addr;

use num_enum::{IntoPrimitive, TryFromPrimitive};
use satrs_core::{
    events::{EventU32TypedSev, SeverityInfo},
    pool::{StaticMemoryPool, StaticPoolConfig},
};

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
    AcsSubsystem = 7,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum TcReceiverId {
    PusTest = 1,
    PusEvent = 2,
    PusHk = 3,
    PusAction = 4,
    PusSched = 5,
}
pub mod pool {
    use super::*;
    pub fn create_static_pools() -> (StaticMemoryPool, StaticMemoryPool) {
        (
            StaticMemoryPool::new(StaticPoolConfig::new(
                vec![
                    (30, 32),
                    (15, 64),
                    (15, 128),
                    (15, 256),
                    (15, 1024),
                    (15, 2048),
                ],
                true,
            )),
            StaticMemoryPool::new(StaticPoolConfig::new(
                vec![
                    (30, 32),
                    (15, 64),
                    (15, 128),
                    (15, 256),
                    (15, 1024),
                    (15, 2048),
                ],
                true,
            )),
        )
    }

    pub fn create_sched_tc_pool() -> StaticMemoryPool {
        StaticMemoryPool::new(StaticPoolConfig::new(
            vec![
                (30, 32),
                (15, 64),
                (15, 128),
                (15, 256),
                (15, 1024),
                (15, 2048),
            ],
            true,
        ))
    }
}

pub mod tasks {
    pub const FREQ_MS_UDP_TMTC: u64 = 200;
    pub const FREQ_MS_EVENT_HANDLING: u64 = 400;
    pub const FREQ_MS_AOCS: u64 = 500;
    pub const FREQ_MS_PUS_STACK: u64 = 200;
}
