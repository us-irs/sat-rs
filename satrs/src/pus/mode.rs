use num_enum::{IntoPrimitive, TryFromPrimitive};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

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
