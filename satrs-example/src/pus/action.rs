use log::{error, warn};
use satrs::action::{ActionRequest, ActionRequestVariant};
use satrs::params::WritableToBeBytes;
use satrs::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs::pus::action::{
    ActionReplyPus, ActionReplyPusWithActionId, ActionRequestWithId, ActivePusActionRequestStd,
    DefaultActiveActionRequestMap,
};
use satrs::pus::verification::{
    self, FailParams, FailParamsWithStep, TcStateAccepted,
    VerificationReporterWithSharedPoolMpscBoundedSender, VerificationReporterWithVecMpscSender,
    VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{
    ActiveRequestProvider, EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter,
    EcssTcInVecConverter, EcssTcReceiverCore, EcssTmSenderCore, EcssTmtcError,
    GenericConversionError, MpscTcReceiver, PusPacketHandlerResult, PusReplyHandler,
    PusServiceHelper, PusTcToRequestConverter, TmAsVecSenderWithId, TmAsVecSenderWithMpsc,
    TmInSharedPoolSenderWithBoundedMpsc, TmInSharedPoolSenderWithId,
};
use satrs::request::{GenericMessage, TargetAndApidId};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::{EcssEnumU16, PusPacket};
use satrs::tmtc::tm_helper::SharedTmPool;
use satrs::ChannelId;
use satrs_example::config::{tmtc_err, TcReceiverId, TmSenderId, PUS_APID};
use std::sync::mpsc::{self};
use std::time::Duration;

use crate::requests::GenericRequestRouter;

use super::{generic_pus_request_timeout_handler, PusTargetedRequestService};

pub struct ActionReplyHandler {
    fail_data_buf: [u8; 128],
}

impl Default for ActionReplyHandler {
    fn default() -> Self {
        Self {
            fail_data_buf: [0; 128],
        }
    }
}

impl PusReplyHandler<ActivePusActionRequestStd, ActionReplyPusWithActionId> for ActionReplyHandler {
    type Error = EcssTmtcError;

    fn handle_unexpected_reply(
        &mut self,
        reply: &GenericMessage<ActionReplyPusWithActionId>,
        _tm_sender: &impl EcssTmSenderCore,
    ) -> Result<(), Self::Error> {
        log::warn!("received unexpected reply for service 8: {reply:?}");
        Ok(())
    }

    fn handle_reply(
        &mut self,
        reply: &satrs::request::GenericMessage<ActionReplyPusWithActionId>,
        active_request: &ActivePusActionRequestStd,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
        _tm_sender: &impl EcssTmSenderCore,
    ) -> Result<bool, Self::Error> {
        let remove_entry = match &reply.message.variant {
            ActionReplyPus::CompletionFailed { error_code, params } => {
                let fail_data_len = params.write_to_be_bytes(&mut self.fail_data_buf)?;
                verification_handler
                    .completion_failure(
                        active_request.token(),
                        FailParams::new(
                            time_stamp,
                            error_code,
                            &self.fail_data_buf[..fail_data_len],
                        ),
                    )
                    .map_err(|e| e.0)?;
                true
            }
            ActionReplyPus::StepFailed {
                error_code,
                step,
                params,
            } => {
                let fail_data_len = params.write_to_be_bytes(&mut self.fail_data_buf)?;
                verification_handler
                    .step_failure(
                        active_request.token(),
                        FailParamsWithStep::new(
                            time_stamp,
                            &EcssEnumU16::new(*step),
                            error_code,
                            &self.fail_data_buf[..fail_data_len],
                        ),
                    )
                    .map_err(|e| e.0)?;
                true
            }
            ActionReplyPus::Completed => {
                verification_handler
                    .completion_success(active_request.token(), time_stamp)
                    .map_err(|e| e.0)?;
                true
            }
            ActionReplyPus::StepSuccess { step } => {
                verification_handler.step_success(
                    &active_request.token(),
                    time_stamp,
                    EcssEnumU16::new(*step),
                )?;
                false
            }
            _ => false,
        };
        Ok(remove_entry)
    }

    fn handle_request_timeout(
        &mut self,
        active_request: &ActivePusActionRequestStd,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
        _tm_sender: &impl EcssTmSenderCore,
    ) -> Result<(), Self::Error> {
        generic_pus_request_timeout_handler(
            active_request,
            verification_handler,
            time_stamp,
            "action",
        )
    }
}

#[derive(Default)]
pub struct ExampleActionRequestConverter {}

impl PusTcToRequestConverter<ActivePusActionRequestStd, ActionRequestWithId>
    for ExampleActionRequestConverter
{
    type Error = GenericConversionError;

    fn convert(
        &mut self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) -> Result<(ActivePusActionRequestStd, ActionRequestWithId), Self::Error> {
        let subservice = tc.subservice();
        let user_data = tc.user_data();
        if user_data.len() < 8 {
            verif_reporter
                .start_failure(
                    token,
                    FailParams::new_no_fail_data(time_stamp, &tmtc_err::NOT_ENOUGH_APP_DATA),
                )
                .expect("Sending start failure failed");
            return Err(GenericConversionError::NotEnoughAppData {
                expected: 8,
                found: user_data.len(),
            });
        }
        let target_id_and_apid = TargetAndApidId::from_pus_tc(tc).unwrap();
        let action_id = u32::from_be_bytes(user_data[4..8].try_into().unwrap());
        if subservice == 128 {
            let token = verif_reporter
                .start_success(token, time_stamp)
                .expect("sending start success verification failed");
            Ok((
                ActivePusActionRequestStd::new(
                    action_id,
                    target_id_and_apid.into(),
                    token,
                    Duration::from_secs(30),
                ),
                ActionRequestWithId {
                    request_id: verification::RequestId::new(tc).into(),
                    request: ActionRequest::new(
                        action_id,
                        ActionRequestVariant::VecData(user_data[8..].to_vec()),
                    ),
                },
            ))
        } else {
            verif_reporter
                .start_failure(
                    token,
                    FailParams::new_no_fail_data(time_stamp, &tmtc_err::INVALID_PUS_SUBSERVICE),
                )
                .expect("Sending start failure failed");
            Err(GenericConversionError::InvalidSubservice(subservice))
        }
    }
}

pub fn create_action_service_static(
    shared_tm_store: SharedTmPool,
    tm_funnel_tx: mpsc::SyncSender<StoreAddr>,
    verif_reporter: VerificationReporterWithSharedPoolMpscBoundedSender,
    tc_pool: SharedStaticMemoryPool,
    pus_action_rx: mpsc::Receiver<EcssTcAndToken>,
    action_router: GenericRequestRouter,
    reply_receiver: mpsc::Receiver<GenericMessage<ActionReplyPusWithActionId>>,
) -> Pus8Wrapper<
    MpscTcReceiver,
    TmInSharedPoolSenderWithBoundedMpsc,
    EcssTcInSharedStoreConverter,
    VerificationReporterWithSharedPoolMpscBoundedSender,
> {
    let action_srv_tm_sender = TmInSharedPoolSenderWithId::new(
        TmSenderId::PusAction as ChannelId,
        "PUS_8_TM_SENDER",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    );
    let action_srv_receiver = MpscTcReceiver::new(
        TcReceiverId::PusAction as ChannelId,
        "PUS_8_TC_RECV",
        pus_action_rx,
    );
    let action_request_handler = PusTargetedRequestService::new(
        PusServiceHelper::new(
            action_srv_receiver,
            action_srv_tm_sender.clone(),
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInSharedStoreConverter::new(tc_pool.clone(), 2048),
        ),
        ExampleActionRequestConverter::default(),
        // TODO: Implementation which does not use run-time allocation? Maybe something like
        // a bounded wrapper which pre-allocates using [HashMap::with_capacity]..
        DefaultActiveActionRequestMap::default(),
        ActionReplyHandler::default(),
        action_router,
        reply_receiver,
    );
    Pus8Wrapper {
        service: action_request_handler,
    }
}

pub fn create_action_service_dynamic(
    tm_funnel_tx: mpsc::Sender<Vec<u8>>,
    verif_reporter: VerificationReporterWithVecMpscSender,
    pus_action_rx: mpsc::Receiver<EcssTcAndToken>,
    action_router: GenericRequestRouter,
    reply_receiver: mpsc::Receiver<GenericMessage<ActionReplyPusWithActionId>>,
) -> Pus8Wrapper<
    MpscTcReceiver,
    TmAsVecSenderWithMpsc,
    EcssTcInVecConverter,
    VerificationReporterWithVecMpscSender,
> {
    let action_srv_tm_sender = TmAsVecSenderWithId::new(
        TmSenderId::PusAction as ChannelId,
        "PUS_8_TM_SENDER",
        tm_funnel_tx.clone(),
    );
    let action_srv_receiver = MpscTcReceiver::new(
        TcReceiverId::PusAction as ChannelId,
        "PUS_8_TC_RECV",
        pus_action_rx,
    );
    let action_request_handler = PusTargetedRequestService::new(
        PusServiceHelper::new(
            action_srv_receiver,
            action_srv_tm_sender.clone(),
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInVecConverter::default(),
        ),
        ExampleActionRequestConverter::default(),
        DefaultActiveActionRequestMap::default(),
        ActionReplyHandler::default(),
        action_router,
        reply_receiver,
    );
    Pus8Wrapper {
        service: action_request_handler,
    }
}

pub struct Pus8Wrapper<
    TcReceiver: EcssTcReceiverCore,
    TmSender: EcssTmSenderCore,
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    pub(crate) service: PusTargetedRequestService<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        ExampleActionRequestConverter,
        ActionReplyHandler,
        DefaultActiveActionRequestMap,
        ActivePusActionRequestStd,
        ActionRequestWithId,
        ActionReplyPusWithActionId,
    >,
}

impl<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > Pus8Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>
{
    pub fn poll_and_handle_next_tc(&mut self, time_stamp: &[u8]) -> bool {
        match self.service.poll_and_handle_next_tc(time_stamp) {
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

    pub fn poll_and_handle_next_reply(&mut self, time_stamp: &[u8]) -> bool {
        match self.service.poll_and_check_next_reply(time_stamp) {
            Ok(packet_handled) => packet_handled,
            Err(e) => {
                log::warn!("PUS 8: Handling reply failed with error {e:?}");
                false
            }
        }
    }
}
