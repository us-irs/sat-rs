use crate::requests::{Request, RequestWithToken};
use log::{error, warn};
use satrs_core::hk::{CollectionIntervalFactor, HkRequest};
use satrs_core::pool::{SharedPool, StoreAddr};
use satrs_core::pus::verification::{
    FailParams, StdVerifReporterWithSender, TcStateAccepted, VerificationToken,
};
use satrs_core::pus::{
    AcceptedTc, EcssTmSender, PusPacketHandlerResult, PusPacketHandlingError, PusServiceBase,
    PusServiceHandler,
};
use satrs_core::spacepackets::ecss::{hk, PusPacket};
use satrs_core::spacepackets::tc::PusTc;
use satrs_core::tmtc::{AddressableId, TargetId};
use satrs_example::{hk_err, tmtc_err};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

pub struct PusService3HkHandler {
    psb: PusServiceBase,
    request_handlers: HashMap<TargetId, Sender<RequestWithToken>>,
}

impl PusService3HkHandler {
    pub fn new(
        receiver: Receiver<AcceptedTc>,
        tc_pool: SharedPool,
        tm_sender: Box<dyn EcssTmSender>,
        tm_apid: u16,
        verification_handler: StdVerifReporterWithSender,
        request_handlers: HashMap<TargetId, Sender<RequestWithToken>>,
    ) -> Self {
        Self {
            psb: PusServiceBase::new(receiver, tc_pool, tm_sender, tm_apid, verification_handler),
            request_handlers,
        }
    }
}

impl PusServiceHandler for PusService3HkHandler {
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
        let user_data = tc.user_data().unwrap();
        if tc.user_data().is_none() {
            self.psb
                .verification_handler
                .borrow_mut()
                .start_failure(
                    token,
                    FailParams::new(Some(&time_stamp), &tmtc_err::NOT_ENOUGH_APP_DATA, None),
                )
                .expect("Sending start failure TM failed");
            return Err(PusPacketHandlingError::NotEnoughAppData(
                "Expected at least 8 bytes of app data".into(),
            ));
        }
        if user_data.len() < 8 {
            let err = if user_data.len() < 4 {
                &hk_err::TARGET_ID_MISSING
            } else {
                &hk_err::UNIQUE_ID_MISSING
            };
            self.psb
                .verification_handler
                .borrow_mut()
                .start_failure(token, FailParams::new(Some(&time_stamp), err, None))
                .expect("Sending start failure TM failed");
            return Err(PusPacketHandlingError::NotEnoughAppData(
                "Expected at least 8 bytes of app data".into(),
            ));
        }
        let addressable_id = AddressableId::from_raw_be(user_data).unwrap();
        if !self
            .request_handlers
            .contains_key(&addressable_id.target_id)
        {
            self.psb
                .verification_handler
                .borrow_mut()
                .start_failure(
                    token,
                    FailParams::new(Some(&time_stamp), &hk_err::UNKNOWN_TARGET_ID, None),
                )
                .expect("Sending start failure TM failed");
            let tgt_id = addressable_id.target_id;
            return Err(PusPacketHandlingError::NotEnoughAppData(format!(
                "Unknown target ID {tgt_id}"
            )));
        }
        let send_request = |target: TargetId, request: HkRequest| {
            let sender = self
                .request_handlers
                .get(&addressable_id.target_id)
                .unwrap();
            sender
                .send(RequestWithToken::new(target, Request::Hk(request), token))
                .unwrap_or_else(|_| panic!("Sending HK request {request:?} failed"));
        };
        if subservice == hk::Subservice::TcEnableHkGeneration as u8 {
            send_request(
                addressable_id.target_id,
                HkRequest::Enable(addressable_id.unique_id),
            );
        } else if subservice == hk::Subservice::TcDisableHkGeneration as u8 {
            send_request(
                addressable_id.target_id,
                HkRequest::Disable(addressable_id.unique_id),
            );
        } else if subservice == hk::Subservice::TcGenerateOneShotHk as u8 {
            send_request(
                addressable_id.target_id,
                HkRequest::OneShot(addressable_id.unique_id),
            );
        } else if subservice == hk::Subservice::TcModifyHkCollectionInterval as u8 {
            if user_data.len() < 12 {
                self.psb
                    .verification_handler
                    .borrow_mut()
                    .start_failure(
                        token,
                        FailParams::new(
                            Some(&time_stamp),
                            &hk_err::COLLECTION_INTERVAL_MISSING,
                            None,
                        ),
                    )
                    .expect("Sending start failure TM failed");
                return Err(PusPacketHandlingError::NotEnoughAppData(
                    "Collection interval missing".into(),
                ));
            }
            send_request(
                addressable_id.target_id,
                HkRequest::ModifyCollectionInterval(
                    addressable_id.unique_id,
                    CollectionIntervalFactor::from_be_bytes(user_data[8..12].try_into().unwrap()),
                ),
            );
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}

pub struct Pus3Wrapper {
    pub(crate) pus_3_handler: PusService3HkHandler,
}

impl Pus3Wrapper {
    pub fn handle_next_packet(&mut self) -> bool {
        match self.pus_3_handler.handle_next_packet() {
            Ok(result) => match result {
                PusPacketHandlerResult::RequestHandled => {}
                PusPacketHandlerResult::RequestHandledPartialSuccess(e) => {
                    warn!("PUS 3 partial packet handling success: {e:?}")
                }
                PusPacketHandlerResult::CustomSubservice(invalid, _) => {
                    warn!("PUS 3 invalid subservice {invalid}");
                }
                PusPacketHandlerResult::SubserviceNotImplemented(subservice, _) => {
                    warn!("PUS 3 subservice {subservice} not implemented");
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
