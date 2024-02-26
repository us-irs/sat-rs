use log::{error, warn};
use satrs::hk::{CollectionIntervalFactor, HkRequest};
use satrs::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs::pus::hk::{PusHkToRequestConverter, PusService3HkHandler};
use satrs::pus::verification::std_mod::{
    VerificationReporterWithSharedPoolMpscBoundedSender, VerificationReporterWithVecMpscSender,
};
use satrs::pus::verification::{
    FailParams, TcStateAccepted, VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter,
    MpscTcReceiver, PusPacketHandlerResult, PusPacketHandlingError, PusServiceHelper,
    TmAsVecSenderWithId, TmInSharedPoolSenderWithId,
};
use satrs::request::TargetAndApidId;
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::{hk, PusPacket};
use satrs::tmtc::tm_helper::SharedTmPool;
use satrs::{ChannelId, TargetId};
use satrs_example::config::{hk_err, tmtc_err, TcReceiverId, TmSenderId, PUS_APID};
use std::sync::mpsc::{self};

use crate::requests::GenericRequestRouter;

use super::GenericRoutingErrorHandler;

#[derive(Default)]
pub struct ExampleHkRequestConverter {}

impl PusHkToRequestConverter for ExampleHkRequestConverter {
    type Error = PusPacketHandlingError;

    fn convert(
        &mut self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) -> Result<(TargetId, HkRequest), Self::Error> {
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
        let target_id = TargetAndApidId::from_pus_tc(tc).expect("invalid tc format");
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
        Ok((
            target_id.into(),
            match standard_subservice.unwrap() {
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
                        CollectionIntervalFactor::from_be_bytes(
                            user_data[8..12].try_into().unwrap(),
                        ),
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
            },
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
) -> Pus3Wrapper<EcssTcInSharedStoreConverter, VerificationReporterWithSharedPoolMpscBoundedSender>
{
    let hk_srv_tm_sender = TmInSharedPoolSenderWithId::new(
        TmSenderId::PusHk as ChannelId,
        "PUS_3_TM_SENDER",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    );
    let hk_srv_receiver =
        MpscTcReceiver::new(TcReceiverId::PusHk as ChannelId, "PUS_8_TC_RECV", pus_hk_rx);
    let pus_3_handler = PusService3HkHandler::new(
        PusServiceHelper::new(
            Box::new(hk_srv_receiver),
            Box::new(hk_srv_tm_sender),
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInSharedStoreConverter::new(tc_pool, 2048),
        ),
        ExampleHkRequestConverter::default(),
        request_router,
        GenericRoutingErrorHandler::default(),
    );
    Pus3Wrapper { pus_3_handler }
}

pub fn create_hk_service_dynamic(
    tm_funnel_tx: mpsc::Sender<Vec<u8>>,
    verif_reporter: VerificationReporterWithVecMpscSender,
    pus_hk_rx: mpsc::Receiver<EcssTcAndToken>,
    request_router: GenericRequestRouter,
) -> Pus3Wrapper<EcssTcInVecConverter, VerificationReporterWithVecMpscSender> {
    let hk_srv_tm_sender = TmAsVecSenderWithId::new(
        TmSenderId::PusHk as ChannelId,
        "PUS_3_TM_SENDER",
        tm_funnel_tx.clone(),
    );
    let hk_srv_receiver =
        MpscTcReceiver::new(TcReceiverId::PusHk as ChannelId, "PUS_8_TC_RECV", pus_hk_rx);
    let pus_3_handler = PusService3HkHandler::new(
        PusServiceHelper::new(
            Box::new(hk_srv_receiver),
            Box::new(hk_srv_tm_sender),
            PUS_APID,
            verif_reporter.clone(),
            EcssTcInVecConverter::default(),
        ),
        ExampleHkRequestConverter::default(),
        request_router,
        GenericRoutingErrorHandler::default(),
    );
    Pus3Wrapper { pus_3_handler }
}

pub struct Pus3Wrapper<
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
> {
    pub(crate) pus_3_handler: PusService3HkHandler<
        TcInMemConverter,
        VerificationReporter,
        ExampleHkRequestConverter,
        GenericRequestRouter,
        GenericRoutingErrorHandler<3>,
    >,
}

impl<
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > Pus3Wrapper<TcInMemConverter, VerificationReporter>
{
    pub fn handle_next_packet(&mut self) -> bool {
        match self.pus_3_handler.handle_one_tc() {
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
