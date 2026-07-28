use num_enum::{IntoPrimitive, TryFromPrimitive};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "alloc")]
#[allow(unused_imports)]
pub use alloc_mod::*;

#[cfg(feature = "std")]
#[allow(unused_imports)]
pub use std_mod::*;

pub const MODE_SERVICE_ID: u8 = 200;

#[derive(Debug, Eq, PartialEq, Copy, Clone, IntoPrimitive, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(u8)]
pub enum Subservice {
    TcSetMode = 1,
    TcReadMode = 3,
    TcAnnounceMode = 4,
    TcAnnounceModeRecursive = 5,
    TmModeReply = 6,
    TmCantReachMode = 7,
    TmWrongModeReply = 8,
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {}

#[cfg(feature = "alloc")]
pub mod std_mod {}

#[cfg(test)]
mod tests {}
