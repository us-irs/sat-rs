use log::{error, warn};
use satrs::action::{ActionRequest, ActionRequestVariant};
use satrs::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs::pus::action::{
    ActionReplyPus, ActionReplyPusWithActionId, ActiveActionRequest, DefaultActiveActionRequestMap,
    PusService8ReplyHandler,
};
use satrs::pus::verification::{
    FailParams, TcStateAccepted, VerificationReporterWithSharedPoolMpscBoundedSender,
    VerificationReporterWithVecMpscSender, VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter,
    EcssTcReceiverCore, EcssTmSenderCore, MpscTcReceiver, PusPacketHandlerResult,
    PusPacketHandlingError, PusReplyHandler, PusServiceHelper, PusTcToRequestConverter,
    ReplyHandlerHook, TmAsVecSenderWithId, TmAsVecSenderWithMpsc,
    TmInSharedPoolSenderWithBoundedMpsc, TmInSharedPoolSenderWithId,
};
use satrs::request::TargetAndApidId;
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusPacket;
use satrs::tmtc::tm_helper::SharedTmPool;
use satrs::{ChannelId, TargetId};
use satrs_example::config::tmtc_err::REQUEST_TIMEOUT;
use satrs_example::config::{tmtc_err, TcReceiverId, TmSenderId, PUS_APID};
use std::sync::mpsc::{self};

use crate::requests::GenericRequestRouter;

use super::PusTargetedRequestService;

#[derive(Default)]
pub struct ActionReplyHandler {}

impl PusReplyHandler<ActiveActionRequest, ActionReplyPusWithActionId> for ActionReplyHandler {
    type Error = ();

    fn handle_unexpected_reply(
        &mut self,
        reply: &satrs::request::GenericMessage<ActionReplyPusWithActionId>,
    ) {
        log::warn!("received unexpected reply for service {SERVICE}: {reply}");
    }

    fn handle_reply(
        &mut self,
        reply: &satrs::request::GenericMessage<ActionReplyPusWithActionId>,
        verification_handler: &impl VerificationReportingProvider,
        tm_sender: &impl EcssTmSenderCore,
    ) -> Result<bool, Self::Error> {
        todo!()
    }

    /*
    fn timeout_callback(&self, active_request: &ActiveRequestType) {
        log::warn!("timeout for active request {active_request} on service {SERVICE}");
    }

    fn timeout_error_code(&self) -> satrs::res_code::ResultU16 {
        REQUEST_TIMEOUT
    }
    */
}

#[derive(Default)]
pub struct ExampleActionRequestConverter {}

impl PusTcToRequestConverter<ActionRequest> for ExampleActionRequestConverter {
    type Error = PusPacketHandlingError;

    fn convert(
        &mut self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) -> Result<(TargetId, ActionRequest), Self::Error> {
        let subservice = tc.subservice();
        let user_data = tc.user_data();
        if user_data.len() < 8 {
            verif_reporter
                .start_failure(
                    token,
                    FailParams::new_no_fail_data(time_stamp, &tmtc_err::NOT_ENOUGH_APP_DATA),
                )
                .expect("Sending start failure failed");
            return Err(PusPacketHandlingError::NotEnoughAppData {
                expected: 8,
                found: user_data.len(),
            });
        }
        let target_id = TargetAndApidId::from_pus_tc(tc).unwrap();
        let action_id = u32::from_be_bytes(user_data[4..8].try_into().unwrap());
        if subservice == 128 {
            Ok((
                target_id.raw(),
                ActionRequest::new(
                    action_id,
                    ActionRequestVariant::VecData(user_data[8..].to_vec()),
                ),
            ))
        } else {
            verif_reporter
                .start_failure(
                    token,
                    FailParams::new_no_fail_data(time_stamp, &tmtc_err::INVALID_PUS_SUBSERVICE),
                )
                .expect("Sending start failure failed");
            Err(PusPacketHandlingError::InvalidSubservice(subservice))
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
        action_router,
        // TODO: Implementation which does not use run-time allocation? Maybe something like
        // a bounded wrapper which pre-allocates using [HashMap::with_capacity]..
        DefaultActiveActionRequestMap::default(),
        ActionReplyHandler::<8>::default(),
    );
    Pus8Wrapper {
        action_request_handler,
    }
}

pub fn create_action_service_dynamic(
    tm_funnel_tx: mpsc::Sender<Vec<u8>>,
    verif_reporter: VerificationReporterWithVecMpscSender,
    pus_action_rx: mpsc::Receiver<EcssTcAndToken>,
    action_router: GenericRequestRouter,
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
        action_router,
        DefaultActiveActionRequestMap::default(),
        ActionReplyHandler::<8>::default(),
    );
    Pus8Wrapper {
        action_request_handler,
    }
}

#[derive(Default)]
pub struct PusActionReplyHook {}

impl ReplyHandlerHook<ActiveActionRequest, ActionReplyPusWithActionId> for PusActionReplyHook {
    fn handle_unexpected_reply(
        &mut self,
        reply: &satrs::request::GenericMessage<ActionReplyPusWithActionId>,
    ) {
        println!("received unexpected action reply {:?}", reply);
    }

    fn timeout_callback(&self, active_request: &ActiveActionRequest) {
        println!("active request {active_request:?} timed out");
    }

    fn timeout_error_code(&self) -> satrs::res_code::ResultU16 {
        todo!()
    }
}
pub struct Pus8Wrapper<
    TcReceiver: EcssTcReceiverCore,
    TmSender: EcssTmSenderCore,
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    pub(crate) action_request_handler: PusTargetedRequestService<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        ExampleActionRequestConverter,
        ActionRequest,
    >,
}

impl<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > Pus8Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>
{
    pub fn handle_next_packet(&mut self) -> bool {
        match self.action_request_handler.handle_one_tc() {
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
