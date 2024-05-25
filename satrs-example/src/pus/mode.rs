use derive_new::new;
use satrs::tmtc::{PacketAsVec, PacketSenderWithSharedPool};
use std::sync::mpsc;
use std::time::Duration;

use crate::requests::GenericRequestRouter;
use satrs::pool::SharedStaticMemoryPool;
use satrs::pus::verification::VerificationReporter;
use satrs::pus::{
    DefaultActiveRequestMap, EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter,
    EcssTcInVecConverter, MpscTcReceiver, MpscTmAsVecSender, PusPacketHandlingError,
    PusServiceHelper,
};
use satrs::request::GenericMessage;
use satrs::{
    mode::{ModeAndSubmode, ModeReply, ModeRequest},
    pus::{
        mode::Subservice,
        verification::{
            self, FailParams, TcStateAccepted, TcStateStarted, VerificationReportingProvider,
            VerificationToken,
        },
        ActivePusRequestStd, ActiveRequestProvider, EcssTmSender, EcssTmtcError,
        GenericConversionError, PusReplyHandler, PusTcToRequestConverter, PusTmVariant,
    },
    request::UniqueApidTargetId,
    spacepackets::{
        ecss::{
            tc::PusTcReader,
            tm::{PusTmCreator, PusTmSecondaryHeader},
            PusPacket,
        },
        SpHeader,
    },
    ComponentId,
};
use satrs_example::config::components::PUS_MODE_SERVICE;
use satrs_example::config::{mode_err, tmtc_err, CustomPusServiceId};

use super::{
    create_verification_reporter, generic_pus_request_timeout_handler, HandlingStatus,
    PusTargetedRequestService, TargetedPusService,
};

#[derive(new)]
pub struct ModeReplyHandler {
    owner_id: ComponentId,
}

impl PusReplyHandler<ActivePusRequestStd, ModeReply> for ModeReplyHandler {
    type Error = EcssTmtcError;

    fn handle_unrequested_reply(
        &mut self,
        reply: &GenericMessage<ModeReply>,
        _tm_sender: &impl EcssTmSender,
    ) -> Result<(), Self::Error> {
        log::warn!("received unexpected reply for mode service 5: {reply:?}");
        Ok(())
    }

    fn handle_reply(
        &mut self,
        reply: &GenericMessage<ModeReply>,
        active_request: &ActivePusRequestStd,
        tm_sender: &impl EcssTmSender,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
    ) -> Result<bool, Self::Error> {
        let started_token: VerificationToken<TcStateStarted> = active_request
            .token()
            .try_into()
            .expect("invalid token state");
        match reply.message {
            ModeReply::ModeReply(mode_reply) => {
                let mut source_data: [u8; 12] = [0; 12];
                mode_reply
                    .write_to_be_bytes(&mut source_data)
                    .expect("writing mode reply failed");
                let req_id = verification::RequestId::from(reply.request_id());
                let sp_header = SpHeader::new_for_unseg_tm(req_id.packet_id().apid(), 0, 0);
                let sec_header =
                    PusTmSecondaryHeader::new(200, Subservice::TmModeReply as u8, 0, 0, time_stamp);
                let pus_tm = PusTmCreator::new(sp_header, sec_header, &source_data, true);
                tm_sender.send_tm(self.owner_id, PusTmVariant::Direct(pus_tm))?;
                verification_handler.completion_success(tm_sender, started_token, time_stamp)?;
            }
            ModeReply::CantReachMode(error_code) => {
                verification_handler.completion_failure(
                    tm_sender,
                    started_token,
                    FailParams::new(time_stamp, &error_code, &[]),
                )?;
            }
            ModeReply::WrongMode { expected, reached } => {
                let mut error_info: [u8; 24] = [0; 24];
                let mut written_len = expected
                    .write_to_be_bytes(&mut error_info[0..ModeAndSubmode::RAW_LEN])
                    .expect("writing expected mode failed");
                written_len += reached
                    .write_to_be_bytes(&mut error_info[ModeAndSubmode::RAW_LEN..])
                    .expect("writing reached mode failed");
                verification_handler.completion_failure(
                    tm_sender,
                    started_token,
                    FailParams::new(
                        time_stamp,
                        &mode_err::WRONG_MODE,
                        &error_info[..written_len],
                    ),
                )?;
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

#[derive(Default)]
pub struct ModeRequestConverter {}

impl PusTcToRequestConverter<ActivePusRequestStd, ModeRequest> for ModeRequestConverter {
    type Error = GenericConversionError;

    fn convert(
        &mut self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        tm_sender: &(impl EcssTmSender + ?Sized),
        verif_reporter: &impl VerificationReportingProvider,
        time_stamp: &[u8],
    ) -> Result<(ActivePusRequestStd, ModeRequest), Self::Error> {
        let subservice = tc.subservice();
        let user_data = tc.user_data();
        let not_enough_app_data = |expected: usize| {
            verif_reporter
                .start_failure(
                    tm_sender,
                    token,
                    FailParams::new_no_fail_data(time_stamp, &tmtc_err::NOT_ENOUGH_APP_DATA),
                )
                .expect("Sending start failure failed");
            Err(GenericConversionError::NotEnoughAppData {
                expected,
                found: user_data.len(),
            })
        };
        if user_data.len() < core::mem::size_of::<u32>() {
            return not_enough_app_data(4);
        }
        let target_id_and_apid = UniqueApidTargetId::from_pus_tc(tc).unwrap();
        let active_request =
            ActivePusRequestStd::new(target_id_and_apid.into(), token, Duration::from_secs(30));
        let subservice_typed = Subservice::try_from(subservice);
        let invalid_subservice = || {
            // Invalid subservice
            verif_reporter
                .start_failure(
                    tm_sender,
                    token,
                    FailParams::new_no_fail_data(time_stamp, &tmtc_err::INVALID_PUS_SUBSERVICE),
                )
                .expect("Sending start failure failed");
            Err(GenericConversionError::InvalidSubservice(subservice))
        };
        if subservice_typed.is_err() {
            return invalid_subservice();
        }
        let subservice_typed = subservice_typed.unwrap();
        match subservice_typed {
            Subservice::TcSetMode => {
                if user_data.len() < core::mem::size_of::<u32>() + ModeAndSubmode::RAW_LEN {
                    return not_enough_app_data(4 + ModeAndSubmode::RAW_LEN);
                }
                let mode_and_submode = ModeAndSubmode::from_be_bytes(&tc.user_data()[4..])
                    .expect("mode and submode extraction failed");
                Ok((active_request, ModeRequest::SetMode(mode_and_submode)))
            }
            Subservice::TcReadMode => Ok((active_request, ModeRequest::ReadMode)),
            Subservice::TcAnnounceMode => Ok((active_request, ModeRequest::AnnounceMode)),
            Subservice::TcAnnounceModeRecursive => {
                Ok((active_request, ModeRequest::AnnounceModeRecursive))
            }
            _ => invalid_subservice(),
        }
    }
}

pub fn create_mode_service_static(
    tm_sender: PacketSenderWithSharedPool,
    tc_pool: SharedStaticMemoryPool,
    pus_action_rx: mpsc::Receiver<EcssTcAndToken>,
    mode_router: GenericRequestRouter,
    reply_receiver: mpsc::Receiver<GenericMessage<ModeReply>>,
) -> ModeServiceWrapper<PacketSenderWithSharedPool, EcssTcInSharedStoreConverter> {
    let mode_request_handler = PusTargetedRequestService::new(
        PusServiceHelper::new(
            PUS_MODE_SERVICE.id(),
            pus_action_rx,
            tm_sender,
            create_verification_reporter(PUS_MODE_SERVICE.id(), PUS_MODE_SERVICE.apid),
            EcssTcInSharedStoreConverter::new(tc_pool, 2048),
        ),
        ModeRequestConverter::default(),
        DefaultActiveRequestMap::default(),
        ModeReplyHandler::new(PUS_MODE_SERVICE.id()),
        mode_router,
        reply_receiver,
    );
    ModeServiceWrapper {
        service: mode_request_handler,
    }
}

pub fn create_mode_service_dynamic(
    tm_funnel_tx: mpsc::Sender<PacketAsVec>,
    pus_action_rx: mpsc::Receiver<EcssTcAndToken>,
    mode_router: GenericRequestRouter,
    reply_receiver: mpsc::Receiver<GenericMessage<ModeReply>>,
) -> ModeServiceWrapper<MpscTmAsVecSender, EcssTcInVecConverter> {
    let mode_request_handler = PusTargetedRequestService::new(
        PusServiceHelper::new(
            PUS_MODE_SERVICE.id(),
            pus_action_rx,
            tm_funnel_tx,
            create_verification_reporter(PUS_MODE_SERVICE.id(), PUS_MODE_SERVICE.apid),
            EcssTcInVecConverter::default(),
        ),
        ModeRequestConverter::default(),
        DefaultActiveRequestMap::default(),
        ModeReplyHandler::new(PUS_MODE_SERVICE.id()),
        mode_router,
        reply_receiver,
    );
    ModeServiceWrapper {
        service: mode_request_handler,
    }
}

pub struct ModeServiceWrapper<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter> {
    pub(crate) service: PusTargetedRequestService<
        MpscTcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        ModeRequestConverter,
        ModeReplyHandler,
        DefaultActiveRequestMap<ActivePusRequestStd>,
        ActivePusRequestStd,
        ModeRequest,
        ModeReply,
    >,
}

impl<TmSender: EcssTmSender, TcInMemConverter: EcssTcInMemConverter> TargetedPusService
    for ModeServiceWrapper<TmSender, TcInMemConverter>
{
    const SERVICE_ID: u8 = CustomPusServiceId::Mode as u8;
    const SERVICE_STR: &'static str = "mode";

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
    use satrs::pus::test_util::{TEST_APID, TEST_COMPONENT_ID_0, TEST_UNIQUE_ID_0};
    use satrs::request::MessageMetadata;
    use satrs::{
        mode::{ModeAndSubmode, ModeReply, ModeRequest},
        pus::mode::Subservice,
        request::GenericMessage,
        spacepackets::{
            ecss::tc::{PusTcCreator, PusTcSecondaryHeader},
            SpHeader,
        },
    };
    use satrs_example::config::tmtc_err;

    use crate::pus::{
        mode::ModeReplyHandler,
        tests::{PusConverterTestbench, ReplyHandlerTestbench},
    };

    use super::ModeRequestConverter;

    #[test]
    fn mode_converter_read_mode_request() {
        let mut testbench =
            PusConverterTestbench::new(TEST_COMPONENT_ID_0.id(), ModeRequestConverter::default());
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header = PusTcSecondaryHeader::new_simple(200, Subservice::TcReadMode as u8);
        let mut app_data: [u8; 4] = [0; 4];
        app_data[0..4].copy_from_slice(&TEST_UNIQUE_ID_0.to_be_bytes());
        let tc = PusTcCreator::new(sp_header, sec_header, &app_data, true);
        let token = testbench.add_tc(&tc);
        let (_active_req, req) = testbench
            .convert(token, &[], TEST_APID, TEST_UNIQUE_ID_0)
            .expect("conversion has failed");
        assert_eq!(req, ModeRequest::ReadMode);
    }

    #[test]
    fn mode_converter_set_mode_request() {
        let mut testbench =
            PusConverterTestbench::new(TEST_COMPONENT_ID_0.id(), ModeRequestConverter::default());
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header = PusTcSecondaryHeader::new_simple(200, Subservice::TcSetMode as u8);
        let mut app_data: [u8; 4 + ModeAndSubmode::RAW_LEN] = [0; 4 + ModeAndSubmode::RAW_LEN];
        let mode_and_submode = ModeAndSubmode::new(2, 1);
        app_data[0..4].copy_from_slice(&TEST_UNIQUE_ID_0.to_be_bytes());
        mode_and_submode
            .write_to_be_bytes(&mut app_data[4..])
            .unwrap();
        let tc = PusTcCreator::new(sp_header, sec_header, &app_data, true);
        let token = testbench.add_tc(&tc);
        let (_active_req, req) = testbench
            .convert(token, &[], TEST_APID, TEST_UNIQUE_ID_0)
            .expect("conversion has failed");
        assert_eq!(req, ModeRequest::SetMode(mode_and_submode));
    }

    #[test]
    fn mode_converter_announce_mode() {
        let mut testbench =
            PusConverterTestbench::new(TEST_COMPONENT_ID_0.id(), ModeRequestConverter::default());
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header = PusTcSecondaryHeader::new_simple(200, Subservice::TcAnnounceMode as u8);
        let mut app_data: [u8; 4] = [0; 4];
        app_data[0..4].copy_from_slice(&TEST_UNIQUE_ID_0.to_be_bytes());
        let tc = PusTcCreator::new(sp_header, sec_header, &app_data, true);
        let token = testbench.add_tc(&tc);
        let (_active_req, req) = testbench
            .convert(token, &[], TEST_APID, TEST_UNIQUE_ID_0)
            .expect("conversion has failed");
        assert_eq!(req, ModeRequest::AnnounceMode);
    }

    #[test]
    fn mode_converter_announce_mode_recursively() {
        let mut testbench =
            PusConverterTestbench::new(TEST_COMPONENT_ID_0.id(), ModeRequestConverter::default());
        let sp_header = SpHeader::new_for_unseg_tc(TEST_APID, 0, 0);
        let sec_header =
            PusTcSecondaryHeader::new_simple(200, Subservice::TcAnnounceModeRecursive as u8);
        let mut app_data: [u8; 4] = [0; 4];
        app_data[0..4].copy_from_slice(&TEST_UNIQUE_ID_0.to_be_bytes());
        let tc = PusTcCreator::new(sp_header, sec_header, &app_data, true);
        let token = testbench.add_tc(&tc);
        let (_active_req, req) = testbench
            .convert(token, &[], TEST_APID, TEST_UNIQUE_ID_0)
            .expect("conversion has failed");
        assert_eq!(req, ModeRequest::AnnounceModeRecursive);
    }

    #[test]
    fn reply_handling_unrequested_reply() {
        let mut testbench = ReplyHandlerTestbench::new(
            TEST_COMPONENT_ID_0.id(),
            ModeReplyHandler::new(TEST_COMPONENT_ID_0.id()),
        );
        let mode_reply = ModeReply::ModeReply(ModeAndSubmode::new(5, 1));
        let unrequested_reply =
            GenericMessage::new(MessageMetadata::new(10_u32, 15_u64), mode_reply);
        // Right now this function does not do a lot. We simply check that it does not panic or do
        // weird stuff.
        let result = testbench.handle_unrequested_reply(&unrequested_reply);
        assert!(result.is_ok());
    }

    #[test]
    fn reply_handling_reply_timeout() {
        let mut testbench = ReplyHandlerTestbench::new(
            TEST_COMPONENT_ID_0.id(),
            ModeReplyHandler::new(TEST_COMPONENT_ID_0.id()),
        );
        let (req_id, active_request) = testbench.add_tc(TEST_APID, TEST_UNIQUE_ID_0, &[]);
        let result = testbench.handle_request_timeout(&active_request, &[]);
        assert!(result.is_ok());
        testbench.verif_reporter.assert_completion_failure(
            TEST_COMPONENT_ID_0.raw(),
            req_id,
            None,
            tmtc_err::REQUEST_TIMEOUT.raw() as u64,
        );
    }
}
