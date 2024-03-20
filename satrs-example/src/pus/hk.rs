use log::{error, warn};
use satrs::hk::{CollectionIntervalFactor, HkRequest};
use satrs::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs::pus::verification::{
    FailParams, TcStateAccepted, VerificationReporterWithSharedPoolMpscBoundedSender,
    VerificationReporterWithVecMpscSender, VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{
    ActivePusRequestStd, ActiveRequestProvider, DefaultActiveRequestMap, EcssTcAndToken,
    EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter, EcssTcReceiverCore,
    EcssTmSenderCore, EcssTmtcError, MpscTcReceiver, PusPacketHandlerResult,
    PusPacketHandlingError, PusReplyHandler, PusServiceHelper, PusTcToRequestConverter,
    TmAsVecSenderWithId, TmAsVecSenderWithMpsc, TmInSharedPoolSenderWithBoundedMpsc,
    TmInSharedPoolSenderWithId,
};
use satrs::request::{TargetAndApidId, GenericMessage};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::{hk, PusPacket};
use satrs::tmtc::tm_helper::SharedTmPool;
use satrs::ChannelId;
use satrs_example::config::{hk_err, tmtc_err, TcReceiverId, TmSenderId, PUS_APID};
use std::sync::mpsc::{self};
use std::time::Duration;

use crate::pus::generic_pus_request_timeout_handler;
use crate::requests::GenericRequestRouter;

use super::PusTargetedRequestService;

#[derive(Clone, PartialEq, Debug)]
pub enum HkReply {
    Ack,
}

#[derive(Default)]
pub struct HkReplyHandler {}

impl PusReplyHandler<ActivePusRequestStd, HkReply> for HkReplyHandler {
    type Error = EcssTmtcError;

    fn handle_unexpected_reply(
        &mut self,
        reply: &satrs::request::GenericMessage<HkReply>,
        _tm_sender: &impl EcssTmSenderCore,
    ) -> Result<(), Self::Error> {
        log::warn!("received unexpected reply for service 3: {reply:?}");
        Ok(())
    }

    fn handle_reply(
        &mut self,
        reply: &satrs::request::GenericMessage<HkReply>,
        active_request: &ActivePusRequestStd,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
        _tm_sender: &impl EcssTmSenderCore,
    ) -> Result<bool, Self::Error> {
        match reply.message {
            HkReply::Ack => {
                verification_handler
                    .completion_success(active_request.token(), time_stamp)
                    .expect("Sending end success TM failed");
            }
        };
        Ok(true)
    }

    fn handle_request_timeout(
        &mut self,
        active_request: &ActivePusRequestStd,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
        _tm_sender: &impl EcssTmSenderCore,
    ) -> Result<(), Self::Error> {
        generic_pus_request_timeout_handler(active_request, verification_handler, time_stamp, "HK")
    }
}

pub struct ExampleHkRequestConverter {
    timeout: Duration,
}

impl Default for ExampleHkRequestConverter {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
        }
    }
}

impl PusTcToRequestConverter<ActivePusRequestStd, HkRequest> for ExampleHkRequestConverter {
    type Error = PusPacketHandlingError;

    fn convert(
        &mut self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) -> Result<(ActivePusRequestStd, HkRequest), Self::Error> {
        let user_data = tc.user_data();
        if user_data.is_empty() {
            let user_data_len = user_data.len() as u32;
            let user_data_len_raw = user_data_len.to_be_bytes();
            verif_reporter
                .start_failure(
                    token,
                    FailParams::new(
                        time_stamp,
                        &tmtc_err::NOT_ENOUGH_APP_DATA,
                        &user_data_len_raw,
                    ),
                )
                .expect("Sending start failure TM failed");
            return Err(PusPacketHandlingError::NotEnoughAppData {
                expected: 4,
                found: 0,
            });
        }
        if user_data.len() < 8 {
            let err = if user_data.len() < 4 {
                &hk_err::TARGET_ID_MISSING
            } else {
                &hk_err::UNIQUE_ID_MISSING
            };
            let user_data_len = user_data.len() as u32;
            let user_data_len_raw = user_data_len.to_be_bytes();
            verif_reporter
                .start_failure(token, FailParams::new(time_stamp, err, &user_data_len_raw))
                .expect("Sending start failure TM failed");
            return Err(PusPacketHandlingError::NotEnoughAppData {
                expected: 8,
                found: 4,
            });
        }
        let subservice = tc.subservice();
        let target_id_and_apid = TargetAndApidId::from_pus_tc(tc).expect("invalid tc format");
        let unique_id = u32::from_be_bytes(tc.user_data()[4..8].try_into().unwrap());

        let standard_subservice = hk::Subservice::try_from(subservice);
        if standard_subservice.is_err() {
            verif_reporter
                .start_failure(
                    token,
                    FailParams::new(time_stamp, &tmtc_err::INVALID_PUS_SUBSERVICE, &[subservice]),
                )
                .expect("Sending start failure TM failed");
            return Err(PusPacketHandlingError::InvalidSubservice(subservice));
        }
        let request = match standard_subservice.unwrap() {
            hk::Subservice::TcEnableHkGeneration | hk::Subservice::TcEnableDiagGeneration => {
                HkRequest::Enable(unique_id)
            }
            hk::Subservice::TcDisableHkGeneration | hk::Subservice::TcDisableDiagGeneration => {
                HkRequest::Disable(unique_id)
            }
            hk::Subservice::TcReportHkReportStructures => todo!(),
            hk::Subservice::TmHkPacket => todo!(),
            hk::Subservice::TcGenerateOneShotHk | hk::Subservice::TcGenerateOneShotDiag => {
                HkRequest::OneShot(unique_id)
            }
            hk::Subservice::TcModifyDiagCollectionInterval
            | hk::Subservice::TcModifyHkCollectionInterval => {
                if user_data.len() < 12 {
                    verif_reporter
                        .start_failure(
                            token,
                            FailParams::new_no_fail_data(
                                time_stamp,
                                &tmtc_err::NOT_ENOUGH_APP_DATA,
                            ),
                        )
                        .expect("Sending start failure TM failed");
                    return Err(PusPacketHandlingError::NotEnoughAppData {
                        expected: 12,
                        found: user_data.len(),
                    });
                }
                HkRequest::ModifyCollectionInterval(
                    unique_id,
                    CollectionIntervalFactor::from_be_bytes(user_data[8..12].try_into().unwrap()),
                )
            }
            _ => {
                verif_reporter
                    .start_failure(
                        token,
                        FailParams::new(
                            time_stamp,
                            &tmtc_err::PUS_SUBSERVICE_NOT_IMPLEMENTED,
                            &[subservice],
                        ),
                    )
                    .expect("Sending start failure TM failed");
                return Err(PusPacketHandlingError::InvalidSubservice(subservice));
            }
        };
        let token = verif_reporter
            .start_success(token, time_stamp)
            .map_err(|e| e.0)?;
        Ok((
            ActivePusRequestStd::new(target_id_and_apid.into(), token, self.timeout),
            request,
        ))
    }
}

pub fn create_hk_service_static(
    shared_tm_store: SharedTmPool,
    tm_funnel_tx: mpsc::SyncSender<StoreAddr>,
    verif_reporter: VerificationReporterWithSharedPoolMpscBoundedSender,
    tc_pool: SharedStaticMemoryPool,
    pus_hk_rx: mpsc::Receiver<EcssTcAndToken>,
    request_router: GenericRequestRouter,
    reply_receiver: mpsc::Receiver<GenericMessage<HkReply>>,
) -> Pus3Wrapper<
    MpscTcReceiver,
    TmInSharedPoolSenderWithBoundedMpsc,
    EcssTcInSharedStoreConverter,
    VerificationReporterWithSharedPoolMpscBoundedSender,
> {
    let hk_srv_tm_sender = TmInSharedPoolSenderWithId::new(
        TmSenderId::PusHk as ChannelId,
        "PUS_3_TM_SENDER",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    );
    let hk_srv_receiver =
        MpscTcReceiver::new(TcReceiverId::PusHk as ChannelId, "PUS_8_TC_RECV", pus_hk_rx);
    let pus_3_handler = PusTargetedRequestService::new(
        PusServiceHelper::new(
            hk_srv_receiver,
            hk_srv_tm_sender,
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInSharedStoreConverter::new(tc_pool, 2048),
        ),
        ExampleHkRequestConverter::default(),
        DefaultActiveRequestMap::default(),
        HkReplyHandler::default(),
        request_router,
        reply_receiver
    );
    Pus3Wrapper { pus_3_handler }
}

pub fn create_hk_service_dynamic(
    tm_funnel_tx: mpsc::Sender<Vec<u8>>,
    verif_reporter: VerificationReporterWithVecMpscSender,
    pus_hk_rx: mpsc::Receiver<EcssTcAndToken>,
    request_router: GenericRequestRouter,
    reply_receiver: mpsc::Receiver<GenericMessage<HkReply>>,
) -> Pus3Wrapper<
    MpscTcReceiver,
    TmAsVecSenderWithMpsc,
    EcssTcInVecConverter,
    VerificationReporterWithVecMpscSender,
> {
    let hk_srv_tm_sender = TmAsVecSenderWithId::new(
        TmSenderId::PusHk as ChannelId,
        "PUS_3_TM_SENDER",
        tm_funnel_tx.clone(),
    );
    let hk_srv_receiver =
        MpscTcReceiver::new(TcReceiverId::PusHk as ChannelId, "PUS_8_TC_RECV", pus_hk_rx);
    let pus_3_handler = PusTargetedRequestService::new(
        PusServiceHelper::new(
            hk_srv_receiver,
            hk_srv_tm_sender,
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInVecConverter::default(),
        ),
        ExampleHkRequestConverter::default(),
        DefaultActiveRequestMap::default(),
        HkReplyHandler::default(),
        request_router,
        reply_receiver
    );
    Pus3Wrapper { pus_3_handler }
}

pub struct Pus3Wrapper<
    TcReceiver: EcssTcReceiverCore,
    TmSender: EcssTmSenderCore,
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    pub(crate) pus_3_handler: PusTargetedRequestService<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        ExampleHkRequestConverter,
        HkReplyHandler,
        DefaultActiveRequestMap<ActivePusRequestStd>,
        ActivePusRequestStd,
        HkRequest,
        HkReply,
    >,
}

impl<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > Pus3Wrapper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>
{
    pub fn handle_next_packet(&mut self, time_stamp: &[u8]) -> bool {
        match self.pus_3_handler.handle_one_tc(time_stamp) {
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
