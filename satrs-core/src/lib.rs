//! # Core components of the sat-rs framework
//!
//! You can find more information about the sat-rs framework on the
//! [homepage](https://egit.irs.uni-stuttgart.de/rust/sat-rs).
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
#![cfg_attr(doc_cfg, feature(doc_cfg))]
#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
extern crate downcast_rs;
#[cfg(any(feature = "std", test))]
extern crate std;

pub mod error;
#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod event_man;
pub mod events;
#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub mod executable;
pub mod hal;
pub mod hk;
pub mod mode;
pub mod objects;
pub mod params;
#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod pool;
pub mod power;
pub mod pus;
pub mod request;
pub mod res_code;
pub mod seq_count;
pub mod tmtc;

pub use spacepackets;

// Generic sender ID type.
pub type SenderId = u32;
