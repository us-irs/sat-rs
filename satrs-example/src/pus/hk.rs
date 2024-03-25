use log::{error, warn};
use satrs::hk::{CollectionIntervalFactor, HkRequest};
use satrs::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs::pus::verification::{
    FailParams, TcStateAccepted, TcStateStarted,
    VerificationReporterWithSharedPoolMpscBoundedSender, VerificationReporterWithVecMpscSender,
    VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{
    ActivePusRequestStd, ActiveRequestProvider, DefaultActiveRequestMap, EcssTcAndToken,
    EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter, EcssTcReceiverCore,
    EcssTmSenderCore, EcssTmtcError, GenericConversionError, MpscTcReceiver,
    PusPacketHandlerResult, PusReplyHandler, PusServiceHelper, PusTcToRequestConverter,
    TmAsVecSenderWithId, TmAsVecSenderWithMpsc, TmInSharedPoolSenderWithBoundedMpsc,
    TmInSharedPoolSenderWithId,
};
use satrs::request::{GenericMessage, TargetAndApidId};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::{hk, PusPacket};
use satrs::tmtc::tm_helper::SharedTmPool;
use satrs::ComponentId;
use satrs_example::config::{hk_err, tmtc_err, ComponentIdList, PUS_APID};
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
        let started_token: VerificationToken<TcStateStarted> = active_request
            .token()
            .try_into()
            .expect("invalid token state");
        match reply.message {
            HkReply::Ack => {
                verification_handler
                    .completion_success(started_token, time_stamp)
                    .expect("sending completio success verification failed");
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
        generic_pus_request_timeout_handler(
            active_request,
            verification_handler,
            time_stamp,
            "HK",
        )?;
        Ok(())
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
    type Error = GenericConversionError;

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
            return Err(GenericConversionError::NotEnoughAppData {
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
            return Err(GenericConversionError::NotEnoughAppData {
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
            return Err(GenericConversionError::InvalidSubservice(subservice));
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
                    return Err(GenericConversionError::NotEnoughAppData {
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
                return Err(GenericConversionError::InvalidSubservice(subservice));
            }
        };
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
        ComponentIdList::PusHk as ComponentId,
        "PUS_3_TM_SENDER",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    );
    let hk_srv_receiver = MpscTcReceiver::new(
        ComponentIdList::PusHk as ComponentId,
        "PUS_8_TC_RECV",
        pus_hk_rx,
    );
    let pus_3_handler = PusTargetedRequestService::new(
        ComponentIdList::PusHk as ComponentId,
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
        reply_receiver,
    );
    Pus3Wrapper {
        service: pus_3_handler,
    }
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
        ComponentIdList::PusHk as ComponentId,
        "PUS_3_TM_SENDER",
        tm_funnel_tx.clone(),
    );
    let hk_srv_receiver = MpscTcReceiver::new(
        ComponentIdList::PusHk as ComponentId,
        "PUS_8_TC_RECV",
        pus_hk_rx,
    );
    let pus_3_handler = PusTargetedRequestService::new(
        ComponentIdList::PusHk as ComponentId,
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
        reply_receiver,
    );
    Pus3Wrapper {
        service: pus_3_handler,
    }
}

pub struct Pus3Wrapper<
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
    pub fn poll_and_handle_next_tc(&mut self, time_stamp: &[u8]) -> bool {
        match self.service.poll_and_handle_next_tc(time_stamp) {
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

    pub fn poll_and_handle_next_reply(&mut self, time_stamp: &[u8]) -> bool {
        match self.service.poll_and_check_next_reply(time_stamp) {
            Ok(packet_handled) => packet_handled,
            Err(e) => {
                log::warn!("PUS 3: Handling reply failed with error {e:?}");
                false
            }
        }
    }

    pub fn check_for_request_timeouts(&mut self) {
        self.service.check_for_request_timeouts();
    }
}

#[cfg(test)]
mod tests {
    use satrs::{
        hk::HkRequest,
        pus::{test_util::TEST_APID, ActiveRequestProvider},
        request::TargetAndApidId,
        spacepackets::{
            ecss::{
                hk::Subservice,
                tc::{PusTcCreator, PusTcReader},
                WritablePusPacket,
            },
            SpHeader,
        },
    };

    use crate::pus::tests::ConverterTestbench;

    use super::ExampleHkRequestConverter;

    #[test]
    fn test_hk_converter_one_shot_req() {
        let mut hk_bench = ConverterTestbench::new(ExampleHkRequestConverter::default());
        let mut sp_header = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();

        let target_id = 2_u32;
        let unique_id = 5_u32;
        let mut app_data: [u8; 8] = [0; 8];
        app_data[0..4].copy_from_slice(&target_id.to_be_bytes());
        app_data[4..8].copy_from_slice(&unique_id.to_be_bytes());

        let hk_req = PusTcCreator::new_simple(
            &mut sp_header,
            3,
            Subservice::TcGenerateOneShotHk as u8,
            Some(&app_data),
            true,
        );
        let accepted_token = hk_bench.add_tc(&hk_req);
        let pus_tc_raw = hk_req.to_vec().unwrap();
        let pus_tc_reader = PusTcReader::new(&pus_tc_raw).expect("invalid pus tc");
        let (active_req, req) = hk_bench
            .convert(accepted_token, &pus_tc_reader.0, &[])
            .expect("conversion failed");
        assert_eq!(
            active_req.target_id(),
            TargetAndApidId::new(TEST_APID, target_id).into()
        );
        if let HkRequest::OneShot(id) = req {
            assert_eq!(id, unique_id);
        } else {
            panic!("unexpected HK request")
        }
    }
}
