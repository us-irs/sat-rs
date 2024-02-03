use crate::requests::{ActionRequest, Request, RequestWithToken};
use log::{error, warn};
use satrs_core::pus::verification::{
    FailParams, TcStateAccepted, VerificationReporterWithSender, VerificationToken,
};
use satrs_core::pus::{
    EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcReceiver, EcssTmSender,
    PusPacketHandlerResult, PusPacketHandlingError, PusServiceBase, PusServiceHelper,
};
use satrs_core::spacepackets::ecss::tc::PusTcReader;
use satrs_core::spacepackets::ecss::PusPacket;
use satrs_example::{tmtc_err, TargetIdWithApid};
use std::collections::HashMap;
use std::sync::mpsc::Sender;

pub struct PusService8ActionHandler<TcInMemConverter: EcssTcInMemConverter> {
    service_helper: PusServiceHelper<TcInMemConverter>,
    request_handlers: HashMap<TargetIdWithApid, Sender<RequestWithToken>>,
}

impl<TcInMemConverter: EcssTcInMemConverter> PusService8ActionHandler<TcInMemConverter> {
    pub fn new(
        tc_receiver: Box<dyn EcssTcReceiver>,
        tm_sender: Box<dyn EcssTmSender>,
        tm_apid: u16,
        verification_handler: VerificationReporterWithSender,
        tc_in_mem_converter: TcInMemConverter,
        request_handlers: HashMap<TargetIdWithApid, Sender<RequestWithToken>>,
    ) -> Self {
        Self {
            service_helper: PusServiceHelper::new(
                tc_receiver,
                tm_sender,
                tm_apid,
                verification_handler,
                tc_in_mem_converter,
            ),
            request_handlers,
        }
    }

    fn handle_action_request_with_id(
        &self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        time_stamp: &[u8],
    ) -> Result<(), PusPacketHandlingError> {
        let user_data = tc.user_data();
        if user_data.len() < 8 {
            self.service_helper
                .common
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
        //let target_id = u32::from_be_bytes(user_data[0..4].try_into().unwrap());
        let target_id = TargetIdWithApid::from_tc(tc).unwrap();
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
            fail_data.copy_from_slice(&target_id.target.to_be_bytes());
            self.service_helper
                .common
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

    fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        let possible_packet = self.service_helper.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(PusPacketHandlerResult::Empty);
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        self.service_helper
            .tc_in_mem_converter
            .cache_ecss_tc_in_memory(&ecss_tc_and_token.tc_in_memory)?;
        let tc = PusTcReader::new(self.service_helper.tc_in_mem_converter.tc_slice_raw())?.0;
        let subservice = tc.subservice();
        let mut partial_error = None;
        let time_stamp = PusServiceBase::get_current_timestamp(&mut partial_error);
        match subservice {
            128 => {
                self.handle_action_request_with_id(ecss_tc_and_token.token, &tc, &time_stamp)?;
            }
            _ => {
                let fail_data = [subservice];
                self.service_helper
                    .common
                    .verification_handler
                    .get_mut()
                    .start_failure(
                        ecss_tc_and_token.token,
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
    pub(crate) pus_8_handler: PusService8ActionHandler<EcssTcInSharedStoreConverter>,
}

impl Pus8Wrapper {
    pub fn handle_next_packet(&mut self) -> bool {
        match self.pus_8_handler.handle_one_tc() {
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
