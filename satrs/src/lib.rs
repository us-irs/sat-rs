//! # sat-rs: A library to build on-board software for remote systems
//!
//! You can find more information about the sat-rs framework on the
//! [homepage](https://absatsw.irs.uni-stuttgart.de/projects/sat-rs/).
//! The [satrs-book](https://absatsw.irs.uni-stuttgart.de/projects/sat-rs/book/) contains
//! high-level information about this framework.
//!
//! ## Overview
//!
//! The core modules of this crate include
//!
//!  - The [event manager][event_man] module which provides a publish and
//!    and subscribe to route events.
//!  - The [pus] module which provides special support for projects using
//!    the [ECSS PUS C standard](https://ecss.nl/standard/ecss-e-st-70-41c-space-engineering-telemetry-and-telecommand-packet-utilization-15-april-2016/).
#![no_std]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#[cfg(any(feature = "alloc", test))]
extern crate alloc;
#[cfg(feature = "alloc")]
extern crate downcast_rs;
#[cfg(any(feature = "std", test))]
extern crate std;

pub mod action;
#[cfg(feature = "alloc")]
pub mod dev_mgmt;
pub mod encoding;
pub mod event_man;
pub mod event_man_legacy;
pub mod events;
pub mod events_legacy;
#[cfg(feature = "std")]
pub mod executable;
pub mod hal;
pub mod health;
pub mod hk;
pub mod mode;
#[cfg(feature = "std")]
pub mod mode_tree;
pub mod params;
pub mod pool;
pub mod power;
pub mod pus;
pub mod queue;
pub mod request;
pub mod res_code;
#[cfg(feature = "alloc")]
pub mod scheduling;
#[cfg(feature = "alloc")]
pub mod subsystem;
pub mod time;
pub mod tmtc;

pub use spacepackets;

use spacepackets::PacketId;

/// Generic component ID type.
pub type ComponentId = u32;

pub trait ValidatorU16Id {
    fn validate(&self, id: u16) -> bool;
}

#[cfg(feature = "alloc")]
impl ValidatorU16Id for alloc::vec::Vec<u16> {
    fn validate(&self, id: u16) -> bool {
        self.contains(&id)
    }
}

#[cfg(feature = "alloc")]
impl ValidatorU16Id for hashbrown::HashSet<u16> {
    fn validate(&self, id: u16) -> bool {
        self.contains(&id)
    }
}

impl ValidatorU16Id for u16 {
    fn validate(&self, id: u16) -> bool {
        id == *self
    }
}

impl ValidatorU16Id for &u16 {
    fn validate(&self, id: u16) -> bool {
        id == **self
    }
}

impl ValidatorU16Id for [u16] {
    fn validate(&self, id: u16) -> bool {
        self.binary_search(&id).is_ok()
    }
}

impl ValidatorU16Id for &[u16] {
    fn validate(&self, id: u16) -> bool {
        self.binary_search(&id).is_ok()
    }
}

#[cfg(feature = "alloc")]
impl ValidatorU16Id for alloc::vec::Vec<spacepackets::PacketId> {
    fn validate(&self, packet_id: u16) -> bool {
        self.contains(&PacketId::from(packet_id))
    }
}

#[cfg(feature = "alloc")]
impl ValidatorU16Id for hashbrown::HashSet<spacepackets::PacketId> {
    fn validate(&self, packet_id: u16) -> bool {
        self.contains(&PacketId::from(packet_id))
    }
}

#[cfg(feature = "std")]
impl ValidatorU16Id for std::collections::HashSet<PacketId> {
    fn validate(&self, packet_id: u16) -> bool {
        self.contains(&PacketId::from(packet_id))
    }
}

impl ValidatorU16Id for [PacketId] {
    fn validate(&self, packet_id: u16) -> bool {
        self.binary_search(&PacketId::from(packet_id)).is_ok()
    }
}

impl ValidatorU16Id for &[PacketId] {
    fn validate(&self, packet_id: u16) -> bool {
        self.binary_search(&PacketId::from(packet_id)).is_ok()
    }
}
