use log::{error, warn};
use satrs::action::{ActionRequest, ActionRequestVariant};
use satrs::params::WritableToBeBytes;
use satrs::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs::pus::action::{
    ActionReplyPus, ActionReplyPusWithActionId, ActivePusActionRequestStd,
    DefaultActiveActionRequestMap,
};
use satrs::pus::verification::{
    FailParams, FailParamsWithStep, TcStateAccepted, TcStateStarted,
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
use satrs::ComponentId;
use satrs_example::config::{tmtc_err, ComponentIdList, PUS_APID};
use std::sync::mpsc::{self};
use std::time::Duration;

use crate::requests::GenericRequestRouter;

use super::{generic_pus_request_timeout_handler, PusTargetedRequestService, TargetedPusService};

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
        let verif_token: VerificationToken<TcStateStarted> = active_request
            .token()
            .try_into()
            .expect("invalid token state");
        let remove_entry = match &reply.message.variant {
            ActionReplyPus::CompletionFailed { error_code, params } => {
                let fail_data_len = params.write_to_be_bytes(&mut self.fail_data_buf)?;
                verification_handler
                    .completion_failure(
                        verif_token,
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
                        verif_token,
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
                    .completion_success(verif_token, time_stamp)
                    .map_err(|e| e.0)?;
                true
            }
            ActionReplyPus::StepSuccess { step } => {
                verification_handler.step_success(
                    &verif_token,
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

impl PusTcToRequestConverter<ActivePusActionRequestStd, ActionRequest>
    for ExampleActionRequestConverter
{
    type Error = GenericConversionError;

    fn convert(
        &mut self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) -> Result<(ActivePusActionRequestStd, ActionRequest), Self::Error> {
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
            let req_variant = if user_data.len() == 8 {
                ActionRequestVariant::NoData
            } else {
                ActionRequestVariant::VecData(user_data[8..].to_vec())
            };
            Ok((
                ActivePusActionRequestStd::new(
                    action_id,
                    target_id_and_apid.into(),
                    token.into(),
                    Duration::from_secs(30),
                ),
                ActionRequest::new(action_id, req_variant),
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
        ComponentIdList::PusAction as ComponentId,
        "PUS_8_TM_SENDER",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    );
    let action_srv_receiver = MpscTcReceiver::new(
        ComponentIdList::PusAction as ComponentId,
        "PUS_8_TC_RECV",
        pus_action_rx,
    );
    let action_request_handler = PusTargetedRequestService::new(
        ComponentIdList::PusAction as ComponentId,
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
        ComponentIdList::PusAction as ComponentId,
        "PUS_8_TM_SENDER",
        tm_funnel_tx.clone(),
    );
    let action_srv_receiver = MpscTcReceiver::new(
        ComponentIdList::PusAction as ComponentId,
        "PUS_8_TC_RECV",
        pus_action_rx,
    );
    let action_request_handler = PusTargetedRequestService::new(
        ComponentIdList::PusAction as ComponentId,
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
        ActionRequest,
        ActionReplyPusWithActionId,
    >,
}

impl<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > TargetedPusService
    for Pus8Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>
{
    /// Returns [true] if the packet handling is finished.
    fn poll_and_handle_next_tc(&mut self, time_stamp: &[u8]) -> bool {
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

    fn poll_and_handle_next_reply(&mut self, time_stamp: &[u8]) -> bool {
        match self.service.poll_and_check_next_reply(time_stamp) {
            Ok(packet_handled) => packet_handled,
            Err(e) => {
                log::warn!("PUS 8: Handling reply failed with error {e:?}");
                false
            }
        }
    }

    fn check_for_request_timeouts(&mut self) {
        self.service.check_for_request_timeouts();
    }
}

#[cfg(test)]
mod tests {
    use satrs::{
        pus::verification::VerificationReporterCfg,
        spacepackets::{
            ecss::{
                tc::{PusTcCreator, PusTcSecondaryHeader},
                tm::PusTmReader,
                WritablePusPacket,
            },
            CcsdsPacket, SpHeader,
        },
    };

    use crate::{
        pus::tests::{TargetedPusRequestTestbench, TARGET_ID, TEST_APID, TEST_APID_TARGET_ID},
        requests::CompositeRequest,
    };

    use super::*;

    impl
        TargetedPusRequestTestbench<
            ExampleActionRequestConverter,
            ActionReplyHandler,
            DefaultActiveActionRequestMap,
            ActivePusActionRequestStd,
            ActionRequest,
            ActionReplyPusWithActionId,
        >
    {
        pub fn new_for_action() -> Self {
            let _ = env_logger::builder().is_test(true).try_init();
            let target_and_apid_id = TargetAndApidId::new(TEST_APID, TEST_APID_TARGET_ID);
            let (tm_funnel_tx, tm_funnel_rx) = mpsc::channel();
            let (pus_action_tx, pus_action_rx) = mpsc::channel();
            let (action_reply_tx, action_reply_rx) = mpsc::channel();
            let (action_req_tx, action_req_rx) = mpsc::channel();
            let verif_reporter_cfg = VerificationReporterCfg::new(TEST_APID, 2, 2, 64).unwrap();
            let tm_as_vec_sender =
                TmAsVecSenderWithMpsc::new(1, "VERIF_SENDER", tm_funnel_tx.clone());
            let verif_reporter =
                VerificationReporterWithVecMpscSender::new(&verif_reporter_cfg, tm_as_vec_sender);
            let mut generic_req_router = GenericRequestRouter::default();
            generic_req_router
                .0
                .insert(target_and_apid_id.into(), action_req_tx);
            let action_srv_tm_sender =
                TmAsVecSenderWithId::new(0, "TESTBENCH", tm_funnel_tx.clone());
            let action_srv_receiver = MpscTcReceiver::new(0, "TESTBENCH", pus_action_rx);
            Self {
                service: PusTargetedRequestService::new(
                    0,
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
                    generic_req_router,
                    action_reply_rx,
                ),
                verif_reporter,
                pus_packet_tx: pus_action_tx,
                tm_funnel_rx,
                reply_tx: action_reply_tx,
                request_rx: action_req_rx,
            }
        }

        pub fn verify_packet_verification(&self, subservice: u8) {
            let next_tm = self.tm_funnel_rx.try_recv().unwrap();
            let verif_tm = PusTmReader::new(&next_tm, 7).unwrap().0;
            assert_eq!(verif_tm.apid(), TEST_APID);
            assert_eq!(verif_tm.service(), 1);
            assert_eq!(verif_tm.subservice(), subservice);
        }

        pub fn verify_tm_empty(&self) {
            let packet = self.tm_funnel_rx.try_recv();
            if let Err(mpsc::TryRecvError::Empty) = packet {
            } else {
                let tm = packet.unwrap();
                let unexpected_tm = PusTmReader::new(&tm, 7).unwrap().0;
                panic!("unexpected TM packet {unexpected_tm:?}");
            }
        }

        pub fn verify_next_tc_is_handled_properly(&mut self, time_stamp: &[u8]) {
            let result = self.service.poll_and_handle_next_tc(time_stamp);
            assert!(result.is_ok());
            let result = result.unwrap();
            match result {
                PusPacketHandlerResult::RequestHandled => (),
                _ => panic!("unexpected result {result:?}"),
            }
        }

        pub fn verify_all_tcs_handled(&mut self, time_stamp: &[u8]) {
            let result = self.service.poll_and_handle_next_tc(time_stamp);
            assert!(result.is_ok());
            let result = result.unwrap();
            match result {
                PusPacketHandlerResult::Empty => (),
                _ => panic!("unexpected result {result:?}"),
            }
        }

        pub fn verify_next_reply_is_handled_properly(&mut self, time_stamp: &[u8]) {
            let result = self.service.poll_and_check_next_reply(time_stamp);
            assert!(result.is_ok());
            assert!(!result.unwrap());
        }

        pub fn verify_all_replies_handled(&mut self, time_stamp: &[u8]) {
            let result = self.service.poll_and_check_next_reply(time_stamp);
            assert!(result.is_ok());
            assert!(result.unwrap());
        }

        pub fn add_tc(&mut self, tc: &PusTcCreator) {
            let token = self.verif_reporter.add_tc(tc);
            let accepted_token = self
                .verif_reporter
                .acceptance_success(token, &[0; 7])
                .expect("TC acceptance failed");
            let next_tm = self.tm_funnel_rx.try_recv().unwrap();
            let verif_tm = PusTmReader::new(&next_tm, 7).unwrap().0;
            assert_eq!(verif_tm.apid(), TEST_APID);
            assert_eq!(verif_tm.service(), 1);
            assert_eq!(verif_tm.subservice(), 1);
            if let Err(mpsc::TryRecvError::Empty) = self.tm_funnel_rx.try_recv() {
            } else {
                let unexpected_tm = PusTmReader::new(&next_tm, 7).unwrap().0;
                panic!("unexpected TM packet {unexpected_tm:?}");
            }
            self.pus_packet_tx
                .send(EcssTcAndToken::new(tc.to_vec().unwrap(), accepted_token))
                .unwrap();
        }
    }

    #[test]
    fn test_basic_request() {
        let mut testbench = TargetedPusRequestTestbench::new_for_action();
        // Create a basic action request and verify forwarding.
        let mut sp_header = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(8, 128);
        let action_id = 5_u32;
        let mut app_data: [u8; 8] = [0; 8];
        app_data[0..4].copy_from_slice(&TEST_APID_TARGET_ID.to_be_bytes());
        app_data[4..8].copy_from_slice(&action_id.to_be_bytes());
        let pus8_packet = PusTcCreator::new(&mut sp_header, sec_header, &app_data, true);
        testbench.add_tc(&pus8_packet);
        let time_stamp: [u8; 7] = [0; 7];
        testbench.verify_next_tc_is_handled_properly(&time_stamp);
        testbench.verify_all_tcs_handled(&time_stamp);

        testbench.verify_packet_verification(3);

        let possible_req = testbench.request_rx.try_recv();
        assert!(possible_req.is_ok());
        let req = possible_req.unwrap();
        if let CompositeRequest::Action(action_req) = req.message {
            assert_eq!(action_req.action_id, action_id);
            assert_eq!(action_req.variant, ActionRequestVariant::NoData);
            let action_reply =
                ActionReplyPusWithActionId::new(action_id, ActionReplyPus::Completed);
            testbench
                .reply_tx
                .send(GenericMessage::new(
                    req.request_id,
                    TARGET_ID.into(),
                    action_reply,
                ))
                .unwrap();
        } else {
            panic!("unexpected request type");
        }
        testbench.verify_next_reply_is_handled_properly(&time_stamp);
        testbench.verify_all_replies_handled(&time_stamp);

        testbench.verify_packet_verification(7);
        testbench.verify_tm_empty();
    }

    #[test]
    fn test_basic_request_routing_error() {
        let mut testbench = TargetedPusRequestTestbench::new_for_action();
        // Create a basic action request and verify forwarding.
        let mut sp_header = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(8, 128);
        let action_id = 5_u32;
        let mut app_data: [u8; 8] = [0; 8];
        // Invalid ID, routing should fail.
        app_data[0..4].copy_from_slice(&(TEST_APID_TARGET_ID + 1).to_be_bytes());
        app_data[4..8].copy_from_slice(&action_id.to_be_bytes());
        let pus8_packet = PusTcCreator::new(&mut sp_header, sec_header, &app_data, true);
        testbench.add_tc(&pus8_packet);
        let time_stamp: [u8; 7] = [0; 7];

        let result = testbench.service.poll_and_handle_next_tc(&time_stamp);
        assert!(result.is_err());
        // Verify the correct result and completion failure.
    }
}
