use num_enum::{IntoPrimitive, TryFromPrimitive};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{mode::ModeRequest, TargetId};

use super::verification::{TcStateAccepted, VerificationToken};

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
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod alloc_mod {}

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub mod std_mod {
    use crate::{
        mode::ModeRequest,
        pus::{GenericRoutingError, PusTargetedRequestHandler},
    };

    pub type PusModeServiceRequestHandler<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        RequestConverter,
        RequestRouter,
        RoutingErrorHandler,
        RoutingError = GenericRoutingError,
    > = PusTargetedRequestHandler<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        RequestConverter,
        RequestRouter,
        RoutingErrorHandler,
        ModeRequest,
        RoutingError,
    >;
}
