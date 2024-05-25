use derive_new::new;
use satrs::hk::{CollectionIntervalFactor, HkRequest, HkRequestVariant, UniqueId};
use satrs::pool::SharedStaticMemoryPool;
use satrs::pus::verification::{
    FailParams, TcStateAccepted, TcStateStarted, VerificationReporter,
    VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{
    ActivePusRequestStd, ActiveRequestProvider, DefaultActiveRequestMap, EcssTcAndToken,
    EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter, EcssTmSender,
    EcssTmtcError, GenericConversionError, MpscTcReceiver, MpscTmAsVecSender,
    PusPacketHandlingError, PusReplyHandler, PusServiceHelper, PusTcToRequestConverter,
};
use satrs::request::{GenericMessage, UniqueApidTargetId};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::{hk, PusPacket, PusServiceId};
use satrs::tmtc::{PacketAsVec, PacketSenderWithSharedPool};
use satrs_example::config::components::PUS_HK_SERVICE;
use satrs_example::config::{hk_err, tmtc_err};
use std::sync::mpsc;
use std::time::Duration;

use crate::pus::{create_verification_reporter, generic_pus_request_timeout_handler};
use crate::requests::GenericRequestRouter;

use super::{HandlingStatus, PusTargetedRequestService, TargetedPusService};

#[derive(Clone, PartialEq, Debug, new)]
pub struct HkReply {
    pub unique_id: UniqueId,
    pub variant: HkReplyVariant,
}

#[derive(Clone, PartialEq, Debug)]
pub enum HkReplyVariant {
    Ack,
}

#[derive(Default)]
pub struct HkReplyHandler {}

impl PusReplyHandler<ActivePusRequestStd, HkReply> for HkReplyHandler {
    type Error = EcssTmtcError;

    fn handle_unrequested_reply(
        &mut self,
        reply: &GenericMessage<HkReply>,
        _tm_sender: &impl EcssTmSender,
    ) -> Result<(), Self::Error> {
        log::warn!("received unexpected reply for service 3: {reply:?}");
        Ok(())
    }

    fn handle_reply(
        &mut self,
        reply: &GenericMessage<HkReply>,
        active_request: &ActivePusRequestStd,
        tm_sender: &impl EcssTmSender,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
    ) -> Result<bool, Self::Error> {
        let started_token: VerificationToken<TcStateStarted> = active_request
            .token()
            .try_into()
            .expect("invalid token state");
        match reply.message.variant {
            HkReplyVariant::Ack => {
                verification_handler
                    .completion_success(tm_sender, started_token, time_stamp)
                    .expect("sending completion success verification failed");
            }
        };
        Ok(true)
    }

    fn handle_request_timeout(
        &mut self,
        active_request: &ActivePusRequestStd,
        tm_sender: &impl EcssTmSender,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
    ) -> Result<(), Self::Error> {
        generic_pus_request_timeout_handler(
            tm_sender,
            active_request,
            verification_handler,
            time_stamp,
            "HK",
        )?;
        Ok(())
    }
}

pub struct HkRequestConverter {
    timeout: Duration,
}

impl Default for HkRequestConverter {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
        }
    }
}

impl PusTcToRequestConverter<ActivePusRequestStd, HkRequest> for HkRequestConverter {
    type Error = GenericConversionError;

    fn convert(
        &mut self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        tm_sender: &(impl EcssTmSender + ?Sized),
        verif_reporter: &impl VerificationReportingProvider,
        time_stamp: &[u8],
    ) -> Result<(ActivePusRequestStd, HkRequest), Self::Error> {
        let user_data = tc.user_data();
        if user_data.is_empty() {
            let user_data_len = user_data.len() as u32;
            let user_data_len_raw = user_data_len.to_be_bytes();
            verif_reporter
                .start_failure(
                    tm_sender,
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
                .start_failure(
                    tm_sender,
                    token,
                    FailParams::new(time_stamp, err, &user_data_len_raw),
                )
                .expect("Sending start failure TM failed");
            return Err(GenericConversionError::NotEnoughAppData {
                expected: 8,
                found: 4,
            });
        }
        let subservice = tc.subservice();
        let target_id_and_apid = UniqueApidTargetId::from_pus_tc(tc).expect("invalid tc format");
        let unique_id = u32::from_be_bytes(tc.user_data()[4..8].try_into().unwrap());

        let standard_subservice = hk::Subservice::try_from(subservice);
        if standard_subservice.is_err() {
            verif_reporter
                .start_failure(
                    tm_sender,
                    token,
                    FailParams::new(time_stamp, &tmtc_err::INVALID_PUS_SUBSERVICE, &[subservice]),
                )
                .expect("Sending start failure TM failed");
            return Err(GenericConversionError::InvalidSubservice(subservice));
        }
        let request = match standard_subservice.unwrap() {
            hk::Subservice::TcEnableHkGeneration | hk::Subservice::TcEnableDiagGeneration => {
                HkRequest::new(unique_id, HkRequestVariant::EnablePeriodic)
            }
            hk::Subservice::TcDisableHkGeneration | hk::Subservice::TcDisableDiagGeneration => {
                HkRequest::new(unique_id, HkRequestVariant::DisablePeriodic)
            }
            hk::Subservice::TcReportHkReportStructures => todo!(),
            hk::Subservice::TmHkPacket => todo!(),
            hk::Subservice::TcGenerateOneShotHk | hk::Subservice::TcGenerateOneShotDiag => {
                HkRequest::new(unique_id, HkRequestVariant::OneShot)
            }
            hk::Subservice::TcModifyDiagCollectionInterval
            | hk::Subservice::TcModifyHkCollectionInterval => {
                if user_data.len() < 12 {
                    verif_reporter
                        .start_failure(
                            tm_sender,
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
                HkRequest::new(
                    unique_id,
                    HkRequestVariant::ModifyCollectionInterval(
                        CollectionIntervalFactor::from_be_bytes(
                            user_data[8..12].try_into().unwrap(),
                        ),
                    ),
                )
            }
            _ => {
                verif_reporter
                    .start_failure(
                        tm_sender,
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
    tm_sender: PacketSenderWithSharedPool,
    tc_pool: SharedStaticMemoryPool,
    pus_hk_rx: mpsc::Receiver<EcssTcAndToken>,
    request_router: GenericRequestRouter,
    reply_receiver: mpsc::Receiver<GenericMessage<HkReply>>,
) -> HkServiceWrapper<PacketSenderWithSharedPool, EcssTcInSharedStoreConverter> {
    let pus_3_handler = PusTargetedRequestService::new(
        PusServiceHelper::new(
            PUS_HK_SERVICE.id(),
            pus_hk_rx,
            tm_sender,
            create_verification_reporter(PUS_HK_SERVICE.id(), PUS_HK_SERVICE.apid),
            EcssTcInSharedStoreConverter::new(tc_pool, 2048),
        ),
        HkRequestConverter::default(),
        DefaultActiveRequestMap::default(),
        HkReplyHandler::default(),
        request_router,
        reply_receiver,
    );
    HkServiceWrapper {
        service: pus_3_handler,
    }
}

pub fn create_hk_service_dynamic(
    tm_funnel_tx: mpsc::Sender<PacketAsVec>,
    pus_hk_rx: mpsc::Receiver<EcssTcAndToken>,
    request_router: GenericRequestRouter,
    reply_receiver: mpsc::Receiver<GenericMessage<HkReply>>,
) -> HkServiceWrapper<MpscTmAsVecSender, EcssTcInVecConverter> {
    let pus_3_handler = PusTargetedRequestService::new(
        PusServiceHelper::new(
            PUS_HK_SERVICE.id(),
            pus_hk_rx,
            tm_funnel_tx,
            create_verification_reporter(PUS_HK_SERVICE.id(), PUS_HK_SERVICE.apid),
            EcssTcInVecConverter::default(),
        ),
        HkRequestConverter::default(),
        DefaultActiveRequestMap::default(),
        HkReplyHandler::default(),
        request_router,
        reply_receiver,
    );
    HkServiceWrapper {
        service: pus_3_handler,
    }
}

pub struct HkServiceWrapper<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter> {
    pub(crate) service: PusTargetedRequestService<
        MpscTcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        HkRequestConverter,
        HkReplyHandler,
        DefaultActiveRequestMap<ActivePusRequestStd>,
        ActivePusRequestStd,
        HkRequest,
        HkReply,
    >,
}

impl<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter> TargetedPusService
    for HkServiceWrapper<TmSender, TcInMemConverter>
{
    const SERVICE_ID: u8 = PusServiceId::Housekeeping as u8;
    const SERVICE_STR: &'static str = "housekeeping";

    delegate::delegate! {
        to self.service {
            fn poll_and_handle_next_tc(
                &mut self,
                time_stamp: &[u8],
            ) -> Result<HandlingStatus, PusPacketHandlingError>;

            fn poll_and_handle_next_reply(
                &mut self,
                time_stamp: &[u8],
            ) -> Result<HandlingStatus, EcssTmtcError>;

            fn check_for_request_timeouts(&mut self);
        }
    }
}

#[cfg(test)]
mod tests {
    use satrs::pus::test_util::{
        TEST_COMPONENT_ID_0, TEST_COMPONENT_ID_1, TEST_UNIQUE_ID_0, TEST_UNIQUE_ID_1,
    };
    use satrs::request::MessageMetadata;
    use satrs::{
        hk::HkRequestVariant,
        pus::test_util::TEST_APID,
        request::GenericMessage,
        spacepackets::{
            ecss::{hk::Subservice, tc::PusTcCreator},
            SpHeader,
        },
    };
    use satrs_example::config::tmtc_err;

    use crate::pus::{
        hk::HkReplyVariant,
        tests::{PusConverterTestbench, ReplyHandlerTestbench},
    };

    use super::{HkReply, HkReplyHandler, HkRequestConverter};

    #[test]
    fn hk_converter_one_shot_req() {
        let mut hk_bench =
            PusConverterTestbench::new(TEST_COMPONENT_ID_0.id(), HkRequestConverter::default());
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let target_id = TEST_UNIQUE_ID_0;
        let unique_id = 5_u32;
        let mut app_data: [u8; 8] = [0; 8];
        app_data[0..4].copy_from_slice(&target_id.to_be_bytes());
        app_data[4..8].copy_from_slice(&unique_id.to_be_bytes());

        let hk_req = PusTcCreator::new_simple(
            sp_header,
            3,
            Subservice::TcGenerateOneShotHk as u8,
            &app_data,
            true,
        );
        let accepted_token = hk_bench.add_tc(&hk_req);
        let (_active_req, req) = hk_bench
            .convert(accepted_token, &[], TEST_APID, TEST_UNIQUE_ID_0)
            .expect("conversion failed");

        assert_eq!(req.unique_id, unique_id);
        if let HkRequestVariant::OneShot = req.variant {
        } else {
            panic!("unexpected HK request")
        }
    }

    #[test]
    fn hk_converter_enable_periodic_generation() {
        let mut hk_bench =
            PusConverterTestbench::new(TEST_COMPONENT_ID_0.id(), HkRequestConverter::default());
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let target_id = TEST_UNIQUE_ID_0;
        let unique_id = 5_u32;
        let mut app_data: [u8; 8] = [0; 8];
        app_data[0..4].copy_from_slice(&target_id.to_be_bytes());
        app_data[4..8].copy_from_slice(&unique_id.to_be_bytes());
        let mut generic_check = |tc: &PusTcCreator| {
            let accepted_token = hk_bench.add_tc(tc);
            let (_active_req, req) = hk_bench
                .convert(accepted_token, &[], TEST_APID, TEST_UNIQUE_ID_0)
                .expect("conversion failed");
            assert_eq!(req.unique_id, unique_id);
            if let HkRequestVariant::EnablePeriodic = req.variant {
            } else {
                panic!("unexpected HK request")
            }
        };
        let tc0 = PusTcCreator::new_simple(
            sp_header,
            3,
            Subservice::TcEnableHkGeneration as u8,
            &app_data,
            true,
        );
        generic_check(&tc0);
        let tc1 = PusTcCreator::new_simple(
            sp_header,
            3,
            Subservice::TcEnableDiagGeneration as u8,
            &app_data,
            true,
        );
        generic_check(&tc1);
    }

    #[test]
    fn hk_conversion_disable_periodic_generation() {
        let mut hk_bench =
            PusConverterTestbench::new(TEST_COMPONENT_ID_0.id(), HkRequestConverter::default());
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let target_id = TEST_UNIQUE_ID_0;
        let unique_id = 5_u32;
        let mut app_data: [u8; 8] = [0; 8];
        app_data[0..4].copy_from_slice(&target_id.to_be_bytes());
        app_data[4..8].copy_from_slice(&unique_id.to_be_bytes());
        let mut generic_check = |tc: &PusTcCreator| {
            let accepted_token = hk_bench.add_tc(tc);
            let (_active_req, req) = hk_bench
                .convert(accepted_token, &[], TEST_APID, TEST_UNIQUE_ID_0)
                .expect("conversion failed");
            assert_eq!(req.unique_id, unique_id);
            if let HkRequestVariant::DisablePeriodic = req.variant {
            } else {
                panic!("unexpected HK request")
            }
        };
        let tc0 = PusTcCreator::new_simple(
            sp_header,
            3,
            Subservice::TcDisableHkGeneration as u8,
            &app_data,
            true,
        );
        generic_check(&tc0);
        let tc1 = PusTcCreator::new_simple(
            sp_header,
            3,
            Subservice::TcDisableDiagGeneration as u8,
            &app_data,
            true,
        );
        generic_check(&tc1);
    }

    #[test]
    fn hk_conversion_modify_interval() {
        let mut hk_bench =
            PusConverterTestbench::new(TEST_COMPONENT_ID_0.id(), HkRequestConverter::default());
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let target_id = TEST_UNIQUE_ID_0;
        let unique_id = 5_u32;
        let mut app_data: [u8; 12] = [0; 12];
        let collection_interval_factor = 5_u32;
        app_data[0..4].copy_from_slice(&target_id.to_be_bytes());
        app_data[4..8].copy_from_slice(&unique_id.to_be_bytes());
        app_data[8..12].copy_from_slice(&collection_interval_factor.to_be_bytes());

        let mut generic_check = |tc: &PusTcCreator| {
            let accepted_token = hk_bench.add_tc(tc);
            let (_active_req, req) = hk_bench
                .convert(accepted_token, &[], TEST_APID, TEST_UNIQUE_ID_0)
                .expect("conversion failed");
            assert_eq!(req.unique_id, unique_id);
            if let HkRequestVariant::ModifyCollectionInterval(interval_factor) = req.variant {
                assert_eq!(interval_factor, collection_interval_factor);
            } else {
                panic!("unexpected HK request")
            }
        };
        let tc0 = PusTcCreator::new_simple(
            sp_header,
            3,
            Subservice::TcModifyHkCollectionInterval as u8,
            &app_data,
            true,
        );
        generic_check(&tc0);
        let tc1 = PusTcCreator::new_simple(
            sp_header,
            3,
            Subservice::TcModifyDiagCollectionInterval as u8,
            &app_data,
            true,
        );
        generic_check(&tc1);
    }

    #[test]
    fn hk_reply_handler() {
        let mut reply_testbench =
            ReplyHandlerTestbench::new(TEST_COMPONENT_ID_0.id(), HkReplyHandler::default());
        let sender_id = 2_u64;
        let apid_target_id = 3_u32;
        let unique_id = 5_u32;
        let (req_id, active_req) = reply_testbench.add_tc(TEST_APID, apid_target_id, &[]);
        let reply = GenericMessage::new(
            MessageMetadata::new(req_id.into(), sender_id),
            HkReply::new(unique_id, HkReplyVariant::Ack),
        );
        let result = reply_testbench.handle_reply(&reply, &active_req, &[]);
        assert!(result.is_ok());
        assert!(result.unwrap());
        reply_testbench
            .verif_reporter
            .assert_full_completion_success(TEST_COMPONENT_ID_0.raw(), req_id, None);
    }

    #[test]
    fn reply_handling_unrequested_reply() {
        let mut testbench =
            ReplyHandlerTestbench::new(TEST_COMPONENT_ID_1.id(), HkReplyHandler::default());
        let action_reply = HkReply::new(5_u32, HkReplyVariant::Ack);
        let unrequested_reply =
            GenericMessage::new(MessageMetadata::new(10_u32, 15_u64), action_reply);
        // Right now this function does not do a lot. We simply check that it does not panic or do
        // weird stuff.
        let result = testbench.handle_unrequested_reply(&unrequested_reply);
        assert!(result.is_ok());
    }

    #[test]
    fn reply_handling_reply_timeout() {
        let mut testbench =
            ReplyHandlerTestbench::new(TEST_COMPONENT_ID_1.id(), HkReplyHandler::default());
        let (req_id, active_request) = testbench.add_tc(TEST_APID, TEST_UNIQUE_ID_1, &[]);
        let result = testbench.handle_request_timeout(&active_request, &[]);
        assert!(result.is_ok());
        testbench.verif_reporter.assert_completion_failure(
            TEST_COMPONENT_ID_1.raw(),
            req_id,
            None,
            tmtc_err::REQUEST_TIMEOUT.raw() as u64,
        );
    }
}
