use lazy_static::lazy_static;
use satrs::{
    res_code::ResultU16,
    spacepackets::{PacketId, PacketType},
};
use satrs_mib::res_code::ResultU16Info;
use satrs_mib::resultcode;
use std::{collections::HashSet, net::Ipv4Addr};
use strum::IntoEnumIterator;

use num_enum::{IntoPrimitive, TryFromPrimitive};
use satrs::{
    events::{EventU32TypedSev, SeverityInfo},
    pool::{StaticMemoryPool, StaticPoolConfig},
};

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
    Mode = 2,
}

pub const OBSW_SERVER_ADDR: Ipv4Addr = Ipv4Addr::UNSPECIFIED;
pub const SERVER_PORT: u16 = 7301;

pub const TEST_EVENT: EventU32TypedSev<SeverityInfo> = EventU32TypedSev::<SeverityInfo>::new(0, 0);

lazy_static! {
    pub static ref PACKET_ID_VALIDATOR: HashSet<PacketId> = {
        let mut set = HashSet::new();
        for id in components::Apid::iter() {
            set.insert(PacketId::new(PacketType::Tc, true, id as u16));
        }
        set
    };
    pub static ref APID_VALIDATOR: HashSet<u16> = {
        let mut set = HashSet::new();
        for id in components::Apid::iter() {
            set.insert(id as u16);
        }
        set
    };
}

pub mod tmtc_err {

    use super::*;

    #[resultcode]
    pub const INVALID_PUS_SERVICE: ResultU16 = ResultU16::new(GroupId::Tmtc as u8, 0);
    #[resultcode]
    pub const INVALID_PUS_SUBSERVICE: ResultU16 = ResultU16::new(GroupId::Tmtc as u8, 1);
    #[resultcode]
    pub const PUS_SERVICE_NOT_IMPLEMENTED: ResultU16 = ResultU16::new(GroupId::Tmtc as u8, 2);
    #[resultcode]
    pub const PUS_SUBSERVICE_NOT_IMPLEMENTED: ResultU16 = ResultU16::new(GroupId::Tmtc as u8, 3);
    #[resultcode]
    pub const UNKNOWN_TARGET_ID: ResultU16 = ResultU16::new(GroupId::Tmtc as u8, 4);
    #[resultcode]
    pub const ROUTING_ERROR: ResultU16 = ResultU16::new(GroupId::Tmtc as u8, 5);
    #[resultcode(info = "Request timeout for targeted PUS request. P1: Request ID. P2: Target ID")]
    pub const REQUEST_TIMEOUT: ResultU16 = ResultU16::new(GroupId::Tmtc as u8, 6);

    #[resultcode(
        info = "Not enough data inside the TC application data field. Optionally includes: \
          8 bytes of failure data containing 2 failure parameters, \
          P1 (u32 big endian): Expected data length, P2: Found data length"
    )]
    pub const NOT_ENOUGH_APP_DATA: ResultU16 = ResultU16::new(GroupId::Tmtc as u8, 2);

    pub const TMTC_RESULTS: &[ResultU16Info] = &[
        INVALID_PUS_SERVICE_EXT,
        INVALID_PUS_SUBSERVICE_EXT,
        PUS_SERVICE_NOT_IMPLEMENTED_EXT,
        UNKNOWN_TARGET_ID_EXT,
        ROUTING_ERROR_EXT,
        NOT_ENOUGH_APP_DATA_EXT,
    ];
}

pub mod hk_err {

    use super::*;

    #[resultcode]
    pub const TARGET_ID_MISSING: ResultU16 = ResultU16::new(GroupId::Hk as u8, 0);
    #[resultcode]
    pub const UNIQUE_ID_MISSING: ResultU16 = ResultU16::new(GroupId::Hk as u8, 1);
    #[resultcode]
    pub const UNKNOWN_TARGET_ID: ResultU16 = ResultU16::new(GroupId::Hk as u8, 2);
    #[resultcode]
    pub const COLLECTION_INTERVAL_MISSING: ResultU16 = ResultU16::new(GroupId::Hk as u8, 3);

    pub const HK_ERR_RESULTS: &[ResultU16Info] = &[
        TARGET_ID_MISSING_EXT,
        UNKNOWN_TARGET_ID_EXT,
        UNKNOWN_TARGET_ID_EXT,
        COLLECTION_INTERVAL_MISSING_EXT,
    ];
}

pub mod mode_err {
    use super::*;

    #[resultcode]
    pub const WRONG_MODE: ResultU16 = ResultU16::new(GroupId::Mode as u8, 0);
}

pub mod components {
    use satrs::request::UniqueApidTargetId;
    use strum::EnumIter;

    #[derive(Copy, Clone, PartialEq, Eq, EnumIter)]
    pub enum Apid {
        Sched = 1,
        GenericPus = 2,
        Acs = 3,
        Cfdp = 4,
        Tmtc = 5,
        Eps = 6,
    }

    // Component IDs for components with the PUS APID.
    #[derive(Copy, Clone, PartialEq, Eq)]
    pub enum PusId {
        PusEventManagement = 0,
        PusRouting = 1,
        PusTest = 2,
        PusAction = 3,
        PusMode = 4,
        PusHk = 5,
    }

    #[derive(Copy, Clone, PartialEq, Eq)]
    pub enum AcsId {
        Mgm0 = 0,
    }

    #[derive(Copy, Clone, PartialEq, Eq)]
    pub enum EpsId {
        Pcdu = 0,
    }

    #[derive(Copy, Clone, PartialEq, Eq)]
    pub enum TmtcId {
        UdpServer = 0,
        TcpServer = 1,
    }

    pub const PUS_ACTION_SERVICE: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::GenericPus as u16, PusId::PusAction as u32);
    pub const PUS_EVENT_MANAGEMENT: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::GenericPus as u16, 0);
    pub const PUS_ROUTING_SERVICE: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::GenericPus as u16, PusId::PusRouting as u32);
    pub const PUS_TEST_SERVICE: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::GenericPus as u16, PusId::PusTest as u32);
    pub const PUS_MODE_SERVICE: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::GenericPus as u16, PusId::PusMode as u32);
    pub const PUS_HK_SERVICE: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::GenericPus as u16, PusId::PusHk as u32);
    pub const PUS_SCHED_SERVICE: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::Sched as u16, 0);
    pub const MGM_HANDLER_0: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::Acs as u16, AcsId::Mgm0 as u32);
    pub const PCDU_HANDLER: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::Eps as u16, EpsId::Pcdu as u32);
    pub const UDP_SERVER: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::Tmtc as u16, TmtcId::UdpServer as u32);
    pub const TCP_SERVER: UniqueApidTargetId =
        UniqueApidTargetId::new(Apid::Tmtc as u16, TmtcId::TcpServer as u32);
}

pub mod pool {
    use super::*;
    pub fn create_static_pools() -> (StaticMemoryPool, StaticMemoryPool) {
        (
            StaticMemoryPool::new(StaticPoolConfig::new_from_subpool_cfg_tuples(
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
            StaticMemoryPool::new(StaticPoolConfig::new_from_subpool_cfg_tuples(
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
        StaticMemoryPool::new(StaticPoolConfig::new_from_subpool_cfg_tuples(
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
    pub const FREQ_MS_AOCS: u64 = 500;
    pub const FREQ_MS_PUS_STACK: u64 = 200;
    pub const SIM_CLIENT_IDLE_DELAY_MS: u64 = 5;
}
