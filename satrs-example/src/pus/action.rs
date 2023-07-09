use crate::requests::{ActionRequest, Request, RequestWithToken};
use log::{error, warn};
use satrs_core::pool::{SharedPool, StoreAddr};
use satrs_core::pus::verification::{
    FailParams, StdVerifReporterWithSender, TcStateAccepted, VerificationToken,
};
use satrs_core::pus::{
    EcssTcReceiver, EcssTmSender, PusPacketHandlerResult, PusPacketHandlingError, PusServiceBase,
    PusServiceHandler,
};
use satrs_core::spacepackets::ecss::tc::PusTc;
use satrs_core::spacepackets::ecss::PusPacket;
use satrs_core::tmtc::TargetId;
use satrs_example::tmtc_err;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

pub struct PusService8ActionHandler {
    psb: PusServiceBase,
    request_handlers: HashMap<TargetId, Sender<RequestWithToken>>,
}

impl PusService8ActionHandler {
    pub fn new(
        tc_receiver: Box<dyn EcssTcReceiver>,
        shared_tc_pool: SharedPool,
        tm_sender: Box<dyn EcssTmSender>,
        tm_apid: u16,
        verification_handler: StdVerifReporterWithSender,
        request_handlers: HashMap<TargetId, Sender<RequestWithToken>>,
    ) -> Self {
        Self {
            psb: PusServiceBase::new(
                tc_receiver,
                shared_tc_pool,
                tm_sender,
                tm_apid,
                verification_handler,
            ),
            request_handlers,
        }
    }
}

impl PusService8ActionHandler {
    fn handle_action_request_with_id(
        &self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTc,
        time_stamp: &[u8],
    ) -> Result<(), PusPacketHandlingError> {
        let user_data = tc.user_data();
        if user_data.is_none() || user_data.unwrap().len() < 8 {
            self.psb()
                .verification_handler
                .borrow_mut()
                .start_failure(
                    token,
                    FailParams::new(Some(time_stamp), &tmtc_err::NOT_ENOUGH_APP_DATA, None),
                )
                .expect("Sending start failure failed");
            return Err(PusPacketHandlingError::NotEnoughAppData(
                "Expected at least 4 bytes".into(),
            ));
        }
        let user_data = user_data.unwrap();
        let target_id = u32::from_be_bytes(user_data[0..4].try_into().unwrap());
        let action_id = u32::from_be_bytes(user_data[4..8].try_into().unwrap());
        if let Some(sender) = self.request_handlers.get(&target_id) {
            sender
                .send(RequestWithToken::new(
                    target_id,
                    Request::Action(ActionRequest::CmdWithU32Id((
                        action_id,
                        Vec::from(&user_data[8..]),
                    ))),
                    token,
                ))
                .expect("Forwarding action request failed");
        } else {
            let mut fail_data: [u8; 4] = [0; 4];
            fail_data.copy_from_slice(&target_id.to_be_bytes());
            self.psb()
                .verification_handler
                .borrow_mut()
                .start_failure(
                    token,
                    FailParams::new(
                        Some(time_stamp),
                        &tmtc_err::UNKNOWN_TARGET_ID,
                        Some(&fail_data),
                    ),
                )
                .expect("Sending start failure failed");
            return Err(PusPacketHandlingError::Other(format!(
                "Unknown target ID {target_id}"
            )));
        }
        Ok(())
    }
}

impl PusServiceHandler for PusService8ActionHandler {
    fn psb_mut(&mut self) -> &mut PusServiceBase {
        &mut self.psb
    }
    fn psb(&self) -> &PusServiceBase {
        &self.psb
    }

    fn handle_one_tc(
        &mut self,
        addr: StoreAddr,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        self.copy_tc_to_buf(addr)?;
        let (tc, _) = PusTc::from_bytes(&self.psb().pus_buf).unwrap();
        let subservice = tc.subservice();
        let mut partial_error = None;
        let time_stamp = self.psb().get_current_timestamp(&mut partial_error);
        match subservice {
            128 => {
                self.handle_action_request_with_id(token, &tc, &time_stamp)?;
            }
            _ => {
                let fail_data = [subservice];
                self.psb_mut()
                    .verification_handler
                    .get_mut()
                    .start_failure(
                        token,
                        FailParams::new(
                            Some(&time_stamp),
                            &tmtc_err::INVALID_PUS_SUBSERVICE,
                            Some(&fail_data),
                        ),
                    )
                    .expect("Sending start failure failed");
                return Err(PusPacketHandlingError::InvalidSubservice(subservice));
            }
        }
        if let Some(partial_error) = partial_error {
            return Ok(PusPacketHandlerResult::RequestHandledPartialSuccess(
                partial_error,
            ));
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}

pub struct Pus8Wrapper {
    pub(crate) pus_8_handler: PusService8ActionHandler,
}

impl Pus8Wrapper {
    pub fn handle_next_packet(&mut self) -> bool {
        match self.pus_8_handler.handle_next_packet() {
            Ok(result) => match result {
                PusPacketHandlerResult::RequestHandled => {}
                PusPacketHandlerResult::RequestHandledPartialSuccess(e) => {
                    warn!("PUS 8 partial packet handling success: {e:?}")
                }
                PusPacketHandlerResult::CustomSubservice(invalid, _) => {
                    warn!("PUS 8 invalid subservice {invalid}");
                }
                PusPacketHandlerResult::SubserviceNotImplemented(subservice, _) => {
                    warn!("PUS 8 subservice {subservice} not implemented");
                }
                PusPacketHandlerResult::Empty => {
                    return true;
                }
            },
            Err(error) => {
                error!("PUS packet handling error: {error:?}")
            }
        }
        false
    }
}
