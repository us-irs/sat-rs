use satrs_core::resultcode::ResultU16;
use satrs_macros::resultcode;

#[derive(Debug)]
pub enum GroupId {
    Tmtc = 0,
}

#[resultcode]
pub const INVALID_PUS_SERVICE: ResultU16 = ResultU16::const_new(GroupId::Tmtc as u8, 0);
#[resultcode]
pub const INVALID_PUS_SUBSERVICE: ResultU16 = ResultU16::const_new(GroupId::Tmtc as u8, 1);

#[resultcode(info="Not enough data inside the TC application data field")]
pub const NOT_ENOUGH_APP_DATA: ResultU16 = ResultU16::const_new(GroupId::Tmtc as u8, 2);
