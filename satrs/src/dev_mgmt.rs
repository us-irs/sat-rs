use crate::{
    mode::{ModeAndSubmode, ModeReply, ModeRequest, ModeRequestSender},
    mode_tree::{ModeStoreProvider, ModeStoreVec},
    queue::GenericTargetedMessagingError,
    request::{GenericMessage, RequestId},
    subsystem::ModeTreeHelperState,
    ComponentId,
};

#[derive(Debug, Default)]
pub enum AssemblyHelperResult {
    #[default]
    Idle,
    TargetKeepingViolation(ComponentId),
    ModeCommandingDone,
}

/// A generic helper for manager components which manage child components in a mode tree.
///
/// Mode commands are usually forwarded to all children components transparently.
/// For example, this could be used in an Assembly component which manages multiple redundant
/// child components. It can also be used inside a manager component which only manages one device.
#[derive(Debug, Default)]
pub struct AssemblyCommandingHelper {
    /// The IDs, modes and reply awaition status of all children are tracked in this data
    /// structure.
    pub children_mode_store: ModeStoreVec,
    /// Target mode used for mode commanding.
    pub target_mode: Option<ModeAndSubmode>,
    /// Request ID of active mode commanding request.
    pub active_request_id: Option<RequestId>,
    pub state: ModeTreeHelperState,
}

pub type DevManagerCommandingHelper = AssemblyCommandingHelper;

impl AssemblyCommandingHelper {
    pub fn send_mode_cmd_to_all_children_with_reply_awaition(
        &mut self,
        request_id: RequestId,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
        mode_req_sender: &impl ModeRequestSender,
    ) -> Result<(), GenericTargetedMessagingError> {
        self.target_mode = Some(mode_and_submode);
        for child in self.children_mode_store.0.iter_mut() {
            mode_req_sender.send_mode_request(
                request_id,
                child.id(),
                ModeRequest::SetMode {
                    mode_and_submode,
                    forced,
                },
            )?;
            child.awaiting_reply = true;
        }
        self.active_request_id = Some(request_id);
        self.state = ModeTreeHelperState::ModeCommanding;
        Ok(())
    }

    pub fn send_announce_mode_cmd_to_children(
        &self,
        request_id: RequestId,
        mode_req_sender: &impl ModeRequestSender,
        recursive: bool,
    ) -> Result<(), GenericTargetedMessagingError> {
        let mut request = ModeRequest::AnnounceMode;
        if recursive {
            request = ModeRequest::AnnounceModeRecursive;
        }
        for child in self.children_mode_store.0.iter() {
            mode_req_sender.send_mode_request(request_id, child.id(), request)?;
        }
        Ok(())
    }

    pub fn add_mode_child(&mut self, target_id: ComponentId, mode: ModeAndSubmode) {
        self.children_mode_store.add_component(target_id, mode);
    }

    pub fn count_number_of_children_with_target_mode(&self) -> Option<usize> {
        self.target_mode?;
        let target_mode = self.target_mode.unwrap();
        let mut children_in_target_mode = 0;
        for child in self.children_mode_store.0.iter() {
            if child.mode_and_submode() == target_mode {
                children_in_target_mode += 1;
            }
        }
        Some(children_in_target_mode)
    }

    pub fn handle_mode_reply(
        &mut self,
        mode_reply: &GenericMessage<ModeReply>,
    ) -> AssemblyHelperResult {
        if !self
            .children_mode_store
            .has_component(mode_reply.sender_id())
        {
            return AssemblyHelperResult::Idle;
        }
        let mut generic_mode_reply_handler = |mode_and_submode: Option<ModeAndSubmode>| {
            // Tying the reply awaition to the request ID ensures that something like replies
            // belonging to older requests do not interfere with the completion handling of
            // the mode commanding. This is important for forced mode commands.
            let mut handle_awaition = false;
            if self.state == ModeTreeHelperState::ModeCommanding
                && self.active_request_id.is_some()
                && mode_reply.request_id() == self.active_request_id.unwrap()
            {
                handle_awaition = true;
            }
            let still_awating_replies = self.children_mode_store.mode_reply_handler(
                mode_reply.sender_id(),
                mode_and_submode,
                handle_awaition,
            );
            if self.state == ModeTreeHelperState::TargetKeeping
                && mode_and_submode.is_some()
                && self.target_mode.is_some()
                && mode_and_submode.unwrap() != self.target_mode.unwrap()
            {
                return AssemblyHelperResult::TargetKeepingViolation(mode_reply.sender_id());
            }
            // It is okay to unwrap: If awaition should be handled, the returned value should
            // always be some valid value.
            if self.state == ModeTreeHelperState::ModeCommanding
                && handle_awaition
                && !still_awating_replies.unwrap()
            {
                self.state = ModeTreeHelperState::TargetKeeping;
                self.active_request_id = None;
                return AssemblyHelperResult::ModeCommandingDone;
            }
            AssemblyHelperResult::Idle
        };
        match mode_reply.message {
            ModeReply::ModeInfo(mode_and_submode) | ModeReply::ModeReply(mode_and_submode) => {
                generic_mode_reply_handler(Some(mode_and_submode))
            }
            ModeReply::CantReachMode(_result_u16) => generic_mode_reply_handler(None),
            ModeReply::WrongMode {
                expected: _,
                reached,
            } => generic_mode_reply_handler(Some(reached)),
        }
    }
}
