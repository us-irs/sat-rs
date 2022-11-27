use std::net::Ipv4Addr;

use satrs_core::res_code::ResultU16;
use satrs_macros::resultcode;
use satrs_mib::res_code::ResultU16Info;

#[derive(Debug)]
pub enum GroupId {
    Tmtc = 0,
}

pub const OBSW_SERVER_ADDR: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
pub const SERVER_PORT: u16 = 7301;

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
