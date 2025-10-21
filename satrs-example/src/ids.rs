//! This is an auto-generated configuration module.
use satrs::request::UniqueApidTargetId;

#[derive(Debug, PartialEq, Eq, strum::EnumIter)]
#[bitbybit::bitenum(u11)]
pub enum Apid {
    Sched = 1,
    GenericPus = 2,
    Acs = 3,
    Cfdp = 4,
    Tmtc = 5,
    Eps = 6,
}

pub mod acs {

    #[derive(Debug, PartialEq, Eq)]
    #[bitbybit::bitenum(u21, exhaustive = false)]
    pub enum Id {
        Subsystem = 1,
        Assembly = 2,
        Mgm0 = 3,
        Mgm1 = 4,
    }

    pub const SUBSYSTEM: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Acs.raw_value(), Id::Subsystem.raw_value());
    pub const ASSEMBLY: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Acs.raw_value(), Id::Assembly.raw_value());
    pub const MGM0: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Acs.raw_value(), Id::Mgm0.raw_value());
    pub const MGM1: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Acs.raw_value(), Id::Mgm1.raw_value());
}

pub mod eps {
    #[derive(Debug, PartialEq, Eq)]
    #[bitbybit::bitenum(u21, exhaustive = false)]
    pub enum Id {
        Pcdu = 0,
        Subsystem = 1,
    }

    pub const PCDU: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Eps.raw_value(), Id::Pcdu.raw_value());
    pub const SUBSYSTEM: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Eps.raw_value(), Id::Subsystem.raw_value());
}

pub mod generic_pus {
    #[derive(Debug, PartialEq, Eq)]
    #[bitbybit::bitenum(u21, exhaustive = false)]
    pub enum Id {
        PusEventManagement = 0,
        PusRouting = 1,
        PusTest = 2,
        PusAction = 3,
        PusMode = 4,
        PusHk = 5,
    }

    pub const PUS_EVENT_MANAGEMENT: super::UniqueApidTargetId = super::UniqueApidTargetId::new(
        super::Apid::GenericPus.raw_value(),
        Id::PusEventManagement.raw_value(),
    );
    pub const PUS_ROUTING: super::UniqueApidTargetId = super::UniqueApidTargetId::new(
        super::Apid::GenericPus.raw_value(),
        Id::PusRouting.raw_value(),
    );
    pub const PUS_TEST: super::UniqueApidTargetId = super::UniqueApidTargetId::new(
        super::Apid::GenericPus.raw_value(),
        Id::PusTest.raw_value(),
    );
    pub const PUS_ACTION: super::UniqueApidTargetId = super::UniqueApidTargetId::new(
        super::Apid::GenericPus.raw_value(),
        Id::PusAction.raw_value(),
    );
    pub const PUS_MODE: super::UniqueApidTargetId = super::UniqueApidTargetId::new(
        super::Apid::GenericPus.raw_value(),
        Id::PusMode.raw_value(),
    );
    pub const PUS_HK: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::GenericPus.raw_value(), Id::PusHk.raw_value());
}

pub mod sched {
    #[derive(Debug, PartialEq, Eq)]
    #[bitbybit::bitenum(u21, exhaustive = false)]
    pub enum Id {
        PusSched = 0,
    }

    pub const PUS_SCHED: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Sched.raw_value(), Id::PusSched.raw_value());
}

pub mod tmtc {
    #[derive(Debug, PartialEq, Eq)]
    #[bitbybit::bitenum(u21, exhaustive = false)]
    pub enum Id {
        UdpServer = 0,
        TcpServer = 1,
    }

    pub const UDP_SERVER: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Tmtc.raw_value(), Id::UdpServer.raw_value());
    pub const TCP_SERVER: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Tmtc.raw_value(), Id::TcpServer.raw_value());
}
