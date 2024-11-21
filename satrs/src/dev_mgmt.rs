use crate::{
    mode::{ModeAndSubmode, ModeReply, ModeRequest, ModeRequestSender},
    mode_tree::{ModeStoreProvider, ModeStoreVec},
    queue::{GenericSendError, GenericTargetedMessagingError},
    request::{GenericMessage, RequestId},
    ComponentId,
};
use core::fmt::Debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveModeCommandContext {
    pub target_mode: ModeAndSubmode,
    pub active_request_id: RequestId,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum DevManagerHelperResult {
    #[default]
    Idle,
    Busy,
    ModeCommandingDone(ActiveModeCommandContext),
}

#[derive(Debug)]
pub enum DevManagerHelperError {
    ChildNotInStore,
}

pub trait DevManagerUserHook: Debug {
    fn send_mode_cmd_to_child(
        &self,
        request_id: RequestId,
        target_id: ComponentId,
        mode: ModeAndSubmode,
        forced: bool,
        children_mode_store: &mut ModeStoreVec,
        mode_req_sender: &impl ModeRequestSender,
    ) -> Result<(), GenericSendError>;

    fn send_mode_cmds_to_children(
        &self,
        request_id: RequestId,
        commanded_parent_mode: ModeAndSubmode,
        forced: bool,
        children_mode_store: &mut ModeStoreVec,
        mode_req_sender: &impl ModeRequestSender,
    ) -> Result<(), GenericSendError>;
}

#[derive(Debug, Default)]
pub struct TransparentDevManagerHook {}

impl DevManagerUserHook for TransparentDevManagerHook {
    fn send_mode_cmds_to_children(
        &self,
        request_id: RequestId,
        commanded_parent_mode: ModeAndSubmode,
        forced: bool,
        children_mode_store: &mut ModeStoreVec,
        mode_req_sender: &impl ModeRequestSender,
    ) -> Result<(), GenericSendError> {
        for child in children_mode_store {
            mode_req_sender.send_mode_request(
                request_id,
                child.id(),
                ModeRequest::SetMode {
                    mode_and_submode: commanded_parent_mode,
                    forced,
                },
            )?;
            child.awaiting_reply = true;
        }
        Ok(())
    }

    fn send_mode_cmd_to_child(
        &self,
        request_id: RequestId,
        target_id: ComponentId,
        mode: ModeAndSubmode,
        forced: bool,
        children_mode_store: &mut ModeStoreVec,
        mode_req_sender: &impl ModeRequestSender,
    ) -> Result<(), GenericSendError> {
        let mut_val = children_mode_store
            .get_mut(target_id)
            .ok_or(GenericSendError::TargetDoesNotExist(target_id))?;
        mut_val.awaiting_reply = true;
        mode_req_sender.send_mode_request(
            request_id,
            target_id,
            ModeRequest::SetMode {
                mode_and_submode: mode,
                forced,
            },
        )?;
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DevManagerCommandingState {
    #[default]
    Idle,
    AwaitingReplies(ActiveModeCommandContext),
}

impl DevManagerCommandingState {
    fn new_active_cmd(mode_and_submode: ModeAndSubmode, active_request_id: RequestId) -> Self {
        DevManagerCommandingState::AwaitingReplies(ActiveModeCommandContext {
            target_mode: mode_and_submode,
            active_request_id,
        })
    }
}

/// A generic helper for manager components which manage child components in a mode tree.
///
/// Mode commands are usually forwarded to all children components transparently.
/// For example, this could be used in an Assembly component which manages multiple redundant
/// child components. It can also be used inside a manager component which only manages one device.
#[derive(Debug, Default)]
pub struct DevManagerCommandingHelper<UserHook: DevManagerUserHook> {
    /// The IDs, modes and reply awaition status of all children are tracked in this data
    /// structure.
    pub children_mode_store: ModeStoreVec,
    pub user_hook: UserHook,
    pub state: DevManagerCommandingState,
}

impl<UserHook: DevManagerUserHook> DevManagerCommandingHelper<UserHook> {
    pub fn new(user_hook: UserHook) -> Self {
        Self {
            children_mode_store: Default::default(),
            user_hook,
            state: Default::default(),
        }
    }

    pub fn send_mode_cmd_to_one_child(
        &mut self,
        request_id: RequestId,
        target_id: ComponentId,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
        mode_req_sender: &impl ModeRequestSender,
    ) -> Result<(), GenericSendError> {
        self.state = DevManagerCommandingState::new_active_cmd(mode_and_submode, request_id);
        self.user_hook.send_mode_cmd_to_child(
            request_id,
            target_id,
            mode_and_submode,
            forced,
            &mut self.children_mode_store,
            mode_req_sender,
        )?;
        Ok(())
    }

    pub fn send_mode_cmd_to_all_children(
        &mut self,
        request_id: RequestId,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
        mode_req_sender: &impl ModeRequestSender,
    ) -> Result<(), GenericSendError> {
        self.state = DevManagerCommandingState::new_active_cmd(mode_and_submode, request_id);
        self.user_hook.send_mode_cmds_to_children(
            request_id,
            mode_and_submode,
            forced,
            &mut self.children_mode_store,
            mode_req_sender,
        )?;
        Ok(())
    }

    pub fn target_mode(&self) -> Option<ModeAndSubmode> {
        match self.state {
            DevManagerCommandingState::Idle => None,
            DevManagerCommandingState::AwaitingReplies(context) => Some(context.target_mode),
        }
    }

    pub fn state(&self) -> DevManagerCommandingState {
        self.state
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

    /// Helper method which counts the number of children which have a certain mode.
    pub fn count_number_of_children_with_mode(&self, mode_and_submode: ModeAndSubmode) -> usize {
        let mut children_in_target_mode = 0;
        for child in &self.children_mode_store {
            if child.mode_and_submode() == mode_and_submode {
                children_in_target_mode += 1;
            }
        }
        children_in_target_mode
    }

    pub fn handle_mode_reply(
        &mut self,
        mode_reply: &GenericMessage<ModeReply>,
    ) -> Result<DevManagerHelperResult, DevManagerHelperError> {
        let context = match self.state {
            DevManagerCommandingState::Idle => return Ok(DevManagerHelperResult::Idle),
            DevManagerCommandingState::AwaitingReplies(active_mode_command_context) => {
                Some(active_mode_command_context)
            }
        };
        if !self
            .children_mode_store
            .has_component(mode_reply.sender_id())
        {
            return Err(DevManagerHelperError::ChildNotInStore);
        }
        let mut generic_mode_reply_handler = |mode_and_submode: Option<ModeAndSubmode>| {
            // Tying the reply awaition to the request ID ensures that something like replies
            // belonging to older requests do not interfere with the completion handling of
            // the mode commanding. This is important for forced mode commands.
            let mut handle_awaition = false;
            if let DevManagerCommandingState::AwaitingReplies { .. } = self.state {
                handle_awaition = true;
            }
            let still_awating_replies = self.children_mode_store.mode_reply_handler(
                mode_reply.sender_id(),
                mode_and_submode,
                handle_awaition,
            );
            // It is okay to unwrap: If awaition should be handled, the returned value should
            // always be some valid value.
            if handle_awaition && !still_awating_replies.unwrap() {
                self.state = DevManagerCommandingState::Idle;
                return Ok(DevManagerHelperResult::ModeCommandingDone(context.unwrap()));
            }
            Ok(DevManagerHelperResult::Busy)
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

#[cfg(test)]
mod tests {
    use crate::{
        mode::{tests::ModeReqSenderMock, UNKNOWN_MODE},
        request::MessageMetadata,
    };

    use super::*;

    pub enum ExampleId {
        Id1 = 1,
        Id2 = 2,
    }

    pub enum ExampleMode {
        Mode1 = 1,
        Mode2 = 2,
    }

    #[test]
    fn test_basic() {
        let assy_helper = DevManagerCommandingHelper::new(TransparentDevManagerHook::default());
        assert_eq!(assy_helper.state(), DevManagerCommandingState::Idle);
    }

    #[test]
    fn test_mode_announce() {
        let mut assy_helper = DevManagerCommandingHelper::new(TransparentDevManagerHook::default());
        let mode_req_sender = ModeReqSenderMock::default();
        assy_helper.add_mode_child(ExampleId::Id1 as u64, UNKNOWN_MODE);
        assy_helper.add_mode_child(ExampleId::Id2 as u64, UNKNOWN_MODE);
        assy_helper
            .send_announce_mode_cmd_to_children(1, &mode_req_sender, false)
            .unwrap();
        assert_eq!(mode_req_sender.requests.borrow().len(), 2);
        let mut req = mode_req_sender.requests.borrow_mut().pop_front().unwrap();
        assert_eq!(req.target_id, ExampleId::Id1 as u64);
        assert_eq!(req.request_id, 1);
        assert_eq!(req.request, ModeRequest::AnnounceMode);
        req = mode_req_sender.requests.borrow_mut().pop_front().unwrap();
        assert_eq!(req.target_id, ExampleId::Id2 as u64);
        assert_eq!(req.request_id, 1);
        assert_eq!(req.request, ModeRequest::AnnounceMode);
    }

    #[test]
    fn test_mode_announce_recursive() {
        let mut assy_helper = DevManagerCommandingHelper::new(TransparentDevManagerHook::default());
        let mode_req_sender = ModeReqSenderMock::default();
        assy_helper.add_mode_child(ExampleId::Id1 as u64, UNKNOWN_MODE);
        assy_helper.add_mode_child(ExampleId::Id2 as u64, UNKNOWN_MODE);
        assy_helper
            .send_announce_mode_cmd_to_children(1, &mode_req_sender, true)
            .unwrap();
        assert_eq!(mode_req_sender.requests.borrow().len(), 2);
        let mut req = mode_req_sender.requests.borrow_mut().pop_front().unwrap();
        assert_eq!(req.target_id, ExampleId::Id1 as u64);
        assert_eq!(req.request_id, 1);
        assert_eq!(req.request, ModeRequest::AnnounceModeRecursive);
        req = mode_req_sender.requests.borrow_mut().pop_front().unwrap();
        assert_eq!(req.target_id, ExampleId::Id2 as u64);
        assert_eq!(req.request_id, 1);
        assert_eq!(req.request, ModeRequest::AnnounceModeRecursive);
    }

    #[test]
    fn test_mode_commanding_one_child() {
        let mut dev_mgmt_helper =
            DevManagerCommandingHelper::new(TransparentDevManagerHook::default());
        let mode_req_sender = ModeReqSenderMock::default();
        dev_mgmt_helper.add_mode_child(ExampleId::Id1 as u64, UNKNOWN_MODE);
        let expected_mode = ModeAndSubmode::new(ExampleMode::Mode1 as u32, 0);
        dev_mgmt_helper
            .send_mode_cmd_to_one_child(
                1,
                ExampleId::Id1 as u64,
                expected_mode,
                false,
                &mode_req_sender,
            )
            .unwrap();
        assert_eq!(mode_req_sender.requests.borrow().len(), 1);
        let req = mode_req_sender.requests.borrow_mut().pop_front().unwrap();
        assert_eq!(req.target_id, ExampleId::Id1 as u64);
        assert_eq!(req.request_id, 1);
        assert_eq!(
            req.request,
            ModeRequest::SetMode {
                mode_and_submode: expected_mode,
                forced: false
            }
        );
        matches!(
            dev_mgmt_helper.state(),
            DevManagerCommandingState::AwaitingReplies { .. }
        );
        if let DevManagerCommandingState::AwaitingReplies(ctx) = dev_mgmt_helper.state() {
            assert_eq!(ctx.target_mode, expected_mode);
            assert_eq!(ctx.active_request_id, 1);
        }
        let reply = GenericMessage::new(
            MessageMetadata::new(1, ExampleId::Id1 as u64),
            ModeReply::ModeReply(expected_mode),
        );
        if let DevManagerHelperResult::ModeCommandingDone(ActiveModeCommandContext {
            target_mode,
            active_request_id,
        }) = dev_mgmt_helper.handle_mode_reply(&reply).unwrap()
        {
            assert_eq!(target_mode, expected_mode);
            assert_eq!(active_request_id, 1);
        }
        matches!(dev_mgmt_helper.state(), DevManagerCommandingState::Idle);
    }

    #[test]
    fn test_mode_commanding_multi_child() {
        let mut dev_mgmt_helper =
            DevManagerCommandingHelper::new(TransparentDevManagerHook::default());
        let mode_req_sender = ModeReqSenderMock::default();
        dev_mgmt_helper.add_mode_child(ExampleId::Id1 as u64, UNKNOWN_MODE);
        dev_mgmt_helper.add_mode_child(ExampleId::Id2 as u64, UNKNOWN_MODE);
        let expected_mode = ModeAndSubmode::new(ExampleMode::Mode2 as u32, 0);
        dev_mgmt_helper
            .send_mode_cmd_to_all_children(1, expected_mode, false, &mode_req_sender)
            .unwrap();
        assert_eq!(mode_req_sender.requests.borrow().len(), 2);
        let req = mode_req_sender.requests.borrow_mut().pop_front().unwrap();
        assert_eq!(req.target_id, ExampleId::Id1 as u64);
        assert_eq!(req.request_id, 1);
        assert_eq!(
            req.request,
            ModeRequest::SetMode {
                mode_and_submode: expected_mode,
                forced: false
            }
        );
        let req = mode_req_sender.requests.borrow_mut().pop_front().unwrap();
        assert_eq!(req.target_id, ExampleId::Id2 as u64);
        assert_eq!(req.request_id, 1);
        assert_eq!(
            req.request,
            ModeRequest::SetMode {
                mode_and_submode: expected_mode,
                forced: false
            }
        );
        matches!(
            dev_mgmt_helper.state(),
            DevManagerCommandingState::AwaitingReplies { .. }
        );
        if let DevManagerCommandingState::AwaitingReplies(ctx) = dev_mgmt_helper.state() {
            assert_eq!(ctx.target_mode, expected_mode);
            assert_eq!(ctx.active_request_id, 1);
        }

        let reply = GenericMessage::new(
            MessageMetadata::new(1, ExampleId::Id1 as u64),
            ModeReply::ModeReply(expected_mode),
        );
        assert_eq!(
            dev_mgmt_helper.handle_mode_reply(&reply).unwrap(),
            DevManagerHelperResult::Busy
        );
        let reply = GenericMessage::new(
            MessageMetadata::new(1, ExampleId::Id2 as u64),
            ModeReply::ModeReply(expected_mode),
        );
        if let DevManagerHelperResult::ModeCommandingDone(ActiveModeCommandContext {
            target_mode,
            active_request_id,
        }) = dev_mgmt_helper.handle_mode_reply(&reply).unwrap()
        {
            assert_eq!(target_mode, expected_mode);
            assert_eq!(active_request_id, 1);
        }
        matches!(dev_mgmt_helper.state(), DevManagerCommandingState::Idle);
    }
}
