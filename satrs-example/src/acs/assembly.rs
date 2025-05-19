// TODO: Write the assembly
//

use std::sync::mpsc;

use satrs::{
    dev_mgmt::{DevManagerCommandingHelper, DevManagerHelperResult, TransparentDevManagerHook},
    mode::{
        ModeAndSubmode, ModeError, ModeProvider, ModeReply, ModeReplyReceiver as _,
        ModeReplySender as _, ModeRequest, ModeRequestHandler, ModeRequestReceiver as _,
        ModeRequestorAndHandlerMpscBounded, UNKNOWN_MODE,
    },
    mode_tree::{ModeChild, ModeNode, ModeParent},
    queue::GenericTargetedMessagingError,
    request::{GenericMessage, MessageMetadata},
    ComponentId,
};
use satrs_example::{ids, DeviceMode};

pub type RequestSenderType = mpsc::SyncSender<GenericMessage<ModeRequest>>;
pub type ReplySenderType = mpsc::SyncSender<GenericMessage<ModeReply>>;

// TODO: Needs to perform same functions as the integration test assembly, but also needs
// to track mode changes and health changes of children.
pub struct MgmAssembly {
    pub mode_node: ModeRequestorAndHandlerMpscBounded,
    pub mode_requestor_info: Option<MessageMetadata>,
    pub mode_and_submode: ModeAndSubmode,
    pub commanding_helper: DevManagerCommandingHelper<TransparentDevManagerHook>,
}

impl MgmAssembly {
    pub fn new(mode_node: ModeRequestorAndHandlerMpscBounded) -> Self {
        Self {
            mode_node,
            mode_requestor_info: None,
            mode_and_submode: UNKNOWN_MODE,
            commanding_helper: DevManagerCommandingHelper::new(TransparentDevManagerHook::default()),
        }
    }

    pub const fn id() -> ComponentId {
        ids::acs::MGM_ASSEMBLY.raw()
    }

    pub fn periodic_operation(&mut self) {
        self.check_mode_requests().expect("mode messaging error");
        self.check_mode_replies().expect("mode messaging error");
        // TODO: perform target keeping, check whether children are in correct mode.
    }

    pub fn check_mode_requests(&mut self) -> Result<(), GenericTargetedMessagingError> {
        while let Some(request) = self.mode_node.try_recv_mode_request()? {
            self.handle_mode_request(request).unwrap();
        }
        Ok(())
    }

    pub fn check_mode_replies(&mut self) -> Result<(), ModeError> {
        while let Some(reply_and_id) = self.mode_node.try_recv_mode_reply()? {
            match self.commanding_helper.handle_mode_reply(&reply_and_id) {
                Ok(result) => {
                    if let DevManagerHelperResult::ModeCommandingDone(context) = result {
                        // Complete the mode command.
                        self.mode_and_submode = context.target_mode;
                        self.handle_mode_reached(self.mode_requestor_info)?;
                    }
                }
                Err(err) => match err {
                    satrs::dev_mgmt::DevManagerHelperError::ChildNotInStore => todo!(),
                },
            }
        }
        Ok(())
    }
}

impl ModeNode for MgmAssembly {
    fn id(&self) -> ComponentId {
        Self::id()
    }
}
impl ModeParent for MgmAssembly {
    type Sender = RequestSenderType;

    fn add_mode_child(&mut self, id: ComponentId, request_sender: RequestSenderType) {
        self.mode_node.add_request_target(id, request_sender);
        self.commanding_helper.add_mode_child(id, UNKNOWN_MODE);
    }
}

impl ModeChild for MgmAssembly {
    type Sender = ReplySenderType;

    fn add_mode_parent(&mut self, id: ComponentId, reply_sender: ReplySenderType) {
        self.mode_node.add_reply_target(id, reply_sender);
    }
}

impl ModeProvider for MgmAssembly {
    fn mode_and_submode(&self) -> ModeAndSubmode {
        self.mode_and_submode
    }
}

impl ModeRequestHandler for MgmAssembly {
    type Error = ModeError;
    fn start_transition(
        &mut self,
        requestor: MessageMetadata,
        mode_and_submode: ModeAndSubmode,
        forced: bool,
    ) -> Result<(), Self::Error> {
        // Always accept forced commands and commands to mode OFF.
        if self.commanding_helper.target_mode().is_some()
            && !forced
            && mode_and_submode.mode() != DeviceMode::Off as u32
        {
            return Err(ModeError::Busy);
        }
        self.mode_requestor_info = Some(requestor);
        self.commanding_helper.send_mode_cmd_to_all_children(
            requestor.request_id(),
            mode_and_submode,
            forced,
            &self.mode_node,
        )?;
        Ok(())
    }

    fn announce_mode(&self, requestor_info: Option<MessageMetadata>, recursive: bool) {
        println!(
            "TestAssembly: Announcing mode (recursively: {}): {:?}",
            recursive, self.mode_and_submode
        );
        let request_id = requestor_info.map_or(0, |info| info.request_id());
        self.commanding_helper
            .send_announce_mode_cmd_to_children(request_id, &self.mode_node, recursive)
            .expect("sending mode request failed");
        // TODO: Send announce event.
        log::info!(
            "MGM assembly announcing mode: {:?}",
            self.mode_and_submode()
        );
    }

    fn handle_mode_reached(
        &mut self,
        mode_requestor: Option<MessageMetadata>,
    ) -> Result<(), Self::Error> {
        if let Some(requestor) = mode_requestor {
            self.send_mode_reply(requestor, ModeReply::ModeReply(self.mode_and_submode))?;
        }
        self.announce_mode(mode_requestor, false);
        Ok(())
    }

    fn handle_mode_info(
        &mut self,
        requestor_info: MessageMetadata,
        info: ModeAndSubmode,
    ) -> Result<(), Self::Error> {
        // TODO: Perform mode keeping.
        Ok(())
    }

    fn send_mode_reply(
        &self,
        requestor: MessageMetadata,
        reply: ModeReply,
    ) -> Result<(), Self::Error> {
        self.mode_node.send_mode_reply(requestor, reply)?;
        Ok(())
    }
}
