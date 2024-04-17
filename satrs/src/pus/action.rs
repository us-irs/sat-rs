use crate::{
    action::{ActionId, ActionRequest},
    params::Params,
    request::{GenericMessage, MessageMetadata, RequestId},
};

use satrs_shared::res_code::ResultU16;

#[cfg(feature = "std")]
pub use std_mod::*;

#[cfg(feature = "alloc")]
#[allow(unused_imports)]
pub use alloc_mod::*;

#[derive(Clone, Debug)]
pub struct ActionRequestWithId {
    pub request_id: RequestId,
    pub request: ActionRequest,
}

/// A reply to an action request, but tailored to the PUS standard verification process.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum ActionReplyVariant {
    Completed,
    StepSuccess {
        step: u16,
    },
    CompletionFailed {
        error_code: ResultU16,
        params: Option<Params>,
    },
    StepFailed {
        error_code: ResultU16,
        step: u16,
        params: Option<Params>,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub struct ActionReplyPus {
    pub action_id: ActionId,
    pub variant: ActionReplyVariant,
}

impl ActionReplyPus {
    pub fn new(action_id: ActionId, variant: ActionReplyVariant) -> Self {
        Self { action_id, variant }
    }
}

pub type GenericActionReplyPus = GenericMessage<ActionReplyPus>;

impl GenericActionReplyPus {
    pub fn new_action_reply(
        requestor_info: MessageMetadata,
        action_id: ActionId,
        reply: ActionReplyVariant,
    ) -> Self {
        Self::new(requestor_info, ActionReplyPus::new(action_id, reply))
    }
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use crate::{
        action::ActionRequest,
        queue::GenericTargetedMessagingError,
        request::{
            GenericMessage, MessageReceiver, MessageSender, MessageSenderAndReceiver, RequestId,
        },
        ComponentId,
    };

    use super::ActionReplyPus;

    /// Helper type definition for a mode handler which can handle mode requests.
    pub type ActionRequestHandlerInterface<S, R> =
        MessageSenderAndReceiver<ActionReplyPus, ActionRequest, S, R>;

    impl<S: MessageSender<ActionReplyPus>, R: MessageReceiver<ActionRequest>>
        ActionRequestHandlerInterface<S, R>
    {
        pub fn try_recv_action_request(
            &self,
        ) -> Result<Option<GenericMessage<ActionRequest>>, GenericTargetedMessagingError> {
            self.try_recv_message()
        }

        pub fn send_action_reply(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            reply: ActionReplyPus,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_id, reply)
        }
    }

    /// Helper type defintion for a mode handler object which can send mode requests and receive
    /// mode replies.
    pub type ActionRequestorInterface<S, R> =
        MessageSenderAndReceiver<ActionRequest, ActionReplyPus, S, R>;

    impl<S: MessageSender<ActionRequest>, R: MessageReceiver<ActionReplyPus>>
        ActionRequestorInterface<S, R>
    {
        pub fn try_recv_action_reply(
            &self,
        ) -> Result<Option<GenericMessage<ActionReplyPus>>, GenericTargetedMessagingError> {
            self.try_recv_message()
        }

        pub fn send_action_request(
            &self,
            request_id: RequestId,
            target_id: ComponentId,
            request: ActionRequest,
        ) -> Result<(), GenericTargetedMessagingError> {
            self.send_message(request_id, target_id, request)
        }
    }
}

#[cfg(feature = "std")]
pub mod std_mod {
    use std::sync::mpsc;

    use crate::{
        pus::{
            verification::{self, TcStateToken},
            ActivePusRequestStd, ActiveRequestProvider, DefaultActiveRequestMap,
        },
        ComponentId,
    };

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ActivePusActionRequestStd {
        pub action_id: ActionId,
        common: ActivePusRequestStd,
    }

    impl ActiveRequestProvider for ActivePusActionRequestStd {
        delegate::delegate! {
            to self.common {
                fn target_id(&self) -> ComponentId;
                fn token(&self) -> verification::TcStateToken;
                fn set_token(&mut self, token: verification::TcStateToken);
                fn has_timed_out(&self) -> bool;
                fn timeout(&self) -> core::time::Duration;
            }
        }
    }

    impl ActivePusActionRequestStd {
        pub fn new_from_common_req(action_id: ActionId, common: ActivePusRequestStd) -> Self {
            Self { action_id, common }
        }

        pub fn new(
            action_id: ActionId,
            target_id: ComponentId,
            token: TcStateToken,
            timeout: core::time::Duration,
        ) -> Self {
            Self {
                action_id,
                common: ActivePusRequestStd::new(target_id, token, timeout),
            }
        }
    }
    pub type DefaultActiveActionRequestMap = DefaultActiveRequestMap<ActivePusActionRequestStd>;

    pub type ActionRequestHandlerMpsc = ActionRequestHandlerInterface<
        mpsc::Sender<GenericMessage<ActionReplyPus>>,
        mpsc::Receiver<GenericMessage<ActionRequest>>,
    >;
    pub type ActionRequestHandlerMpscBounded = ActionRequestHandlerInterface<
        mpsc::SyncSender<GenericMessage<ActionReplyPus>>,
        mpsc::Receiver<GenericMessage<ActionRequest>>,
    >;

    pub type ActionRequestorMpsc = ActionRequestorInterface<
        mpsc::Sender<GenericMessage<ActionRequest>>,
        mpsc::Receiver<GenericMessage<ActionReplyPus>>,
    >;
    pub type ActionRequestorBoundedMpsc = ActionRequestorInterface<
        mpsc::SyncSender<GenericMessage<ActionRequest>>,
        mpsc::Receiver<GenericMessage<ActionReplyPus>>,
    >;
}

#[cfg(test)]
mod tests {}
