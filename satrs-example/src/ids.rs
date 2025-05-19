//! This is an auto-generated configuration module.
use satrs::request::UniqueApidTargetId;

#[derive(Debug, Copy, Clone, PartialEq, Eq, strum::EnumIter)]
pub enum Apid {
    Sched = 1,
    GenericPus = 2,
    Acs = 3,
    Cfdp = 4,
    Tmtc = 5,
    Eps = 6,
}

pub mod acs {
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Id {
        Subsystem = 1,
        Assembly = 2,
        Mgm0 = 3,
        Mgm1 = 4,
    }

    pub const SUBSYSTEM: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Acs as u16, Id::Subsystem as u32);
    pub const ASSEMBLY: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Acs as u16, Id::Assembly as u32);
    pub const MGM0: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Acs as u16, Id::Mgm0 as u32);
    pub const MGM1: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Acs as u16, Id::Mgm1 as u32);
}

pub mod eps {
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Id {
        Pcdu = 0,
        Subsystem = 1,
    }

    pub const PCDU: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Eps as u16, Id::Pcdu as u32);
    pub const SUBSYSTEM: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Eps as u16, Id::Subsystem as u32);
}

pub mod generic_pus {
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Id {
        PusEventManagement = 0,
        PusRouting = 1,
        PusTest = 2,
        PusAction = 3,
        PusMode = 4,
        PusHk = 5,
    }

    pub const PUS_EVENT_MANAGEMENT: super::UniqueApidTargetId = super::UniqueApidTargetId::new(
        super::Apid::GenericPus as u16,
        Id::PusEventManagement as u32,
    );
    pub const PUS_ROUTING: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::GenericPus as u16, Id::PusRouting as u32);
    pub const PUS_TEST: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::GenericPus as u16, Id::PusTest as u32);
    pub const PUS_ACTION: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::GenericPus as u16, Id::PusAction as u32);
    pub const PUS_MODE: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::GenericPus as u16, Id::PusMode as u32);
    pub const PUS_HK: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::GenericPus as u16, Id::PusHk as u32);
}

pub mod sched {
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Id {
        PusSched = 0,
    }

    pub const PUS_SCHED: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Sched as u16, Id::PusSched as u32);
}

pub mod tmtc {
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Id {
        UdpServer = 0,
        TcpServer = 1,
    }

    pub const UDP_SERVER: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Tmtc as u16, Id::UdpServer as u32);
    pub const TCP_SERVER: super::UniqueApidTargetId =
        super::UniqueApidTargetId::new(super::Apid::Tmtc as u16, Id::TcpServer as u32);
}
