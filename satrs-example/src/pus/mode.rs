use std::time::Duration;

use satrs::{
    mode::{ModeAndSubmode, ModeReply, ModeRequest},
    pus::{
        mode::Subservice,
        verification::{
            self, FailParams, TcStateAccepted, TcStateStarted, VerificationReportingProvider,
            VerificationToken,
        },
        ActivePusRequestStd, ActiveRequestProvider, EcssTmSenderCore, EcssTmtcError,
        GenericConversionError, PusReplyHandler, PusTcToRequestConverter, PusTmWrapper,
    },
    request::TargetAndApidId,
    spacepackets::{
        ecss::{
            tc::PusTcReader,
            tm::{PusTmCreator, PusTmSecondaryHeader},
            PusPacket,
        },
        SpHeader,
    },
};
use satrs_example::config::{mode_err, tmtc_err};

use super::generic_pus_request_timeout_handler;

#[derive(Default)]
pub struct ModeReplyHandler {}

impl PusReplyHandler<ActivePusRequestStd, ModeReply> for ModeReplyHandler {
    type Error = EcssTmtcError;

    fn handle_unexpected_reply(
        &mut self,
        reply: &satrs::request::GenericMessage<ModeReply>,
        _tm_sender: &impl EcssTmSenderCore,
    ) -> Result<(), Self::Error> {
        log::warn!("received unexpected reply for mode service 5: {reply:?}");
        Ok(())
    }

    fn handle_reply(
        &mut self,
        reply: &satrs::request::GenericMessage<ModeReply>,
        active_request: &ActivePusRequestStd,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
        tm_sender: &impl EcssTmSenderCore,
    ) -> Result<bool, Self::Error> {
        let started_token: VerificationToken<TcStateStarted> = active_request
            .token()
            .try_into()
            .expect("invalid token state");
        match reply.message {
            ModeReply::ModeInfo(info) => {
                log::warn!(
                    "received unrequest mode information for active request {:?}: {:?}",
                    active_request,
                    info
                );
            }
            ModeReply::ModeReply(mode_reply) => {
                let mut source_data: [u8; 12] = [0; 12];
                mode_reply
                    .write_to_be_bytes(&mut source_data)
                    .expect("writing mode reply failed");
                let req_id = verification::RequestId::from(reply.request_id);
                let mut sp_header = SpHeader::tm_unseg(req_id.packet_id().apid(), 0, 0)
                    .expect("generating SP header failed");
                let sec_header = PusTmSecondaryHeader::new(
                    200,
                    Subservice::TmModeReply as u8,
                    0,
                    0,
                    Some(time_stamp),
                );
                let pus_tm = PusTmCreator::new(&mut sp_header, sec_header, &source_data, true);
                tm_sender.send_tm(PusTmWrapper::Direct(pus_tm))?;
                verification_handler
                    .completion_success(started_token, time_stamp)
                    .map_err(|e| e.0)?;
            }
            ModeReply::CantReachMode(error_code) => {
                verification_handler
                    .completion_failure(
                        started_token,
                        FailParams::new(time_stamp, &error_code, &[]),
                    )
                    .map_err(|e| e.0)?;
            }
            ModeReply::WrongMode { expected, reached } => {
                let mut error_info: [u8; 24] = [0; 24];
                let mut written_len = expected
                    .write_to_be_bytes(&mut error_info[0..ModeAndSubmode::RAW_LEN])
                    .expect("writing expected mode failed");
                written_len += reached
                    .write_to_be_bytes(&mut error_info[ModeAndSubmode::RAW_LEN..])
                    .expect("writing reached mode failed");
                verification_handler
                    .completion_failure(
                        started_token,
                        FailParams::new(
                            time_stamp,
                            &mode_err::WRONG_MODE,
                            &error_info[..written_len],
                        ),
                    )
                    .map_err(|e| e.0)?;
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

#[derive(Default)]
pub struct ModeRequestConverter {}

impl PusTcToRequestConverter<ActivePusRequestStd, ModeRequest> for ModeRequestConverter {
    type Error = GenericConversionError;

    fn convert(
        &mut self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) -> Result<(ActivePusRequestStd, ModeRequest), Self::Error> {
        let subservice = tc.subservice();
        let user_data = tc.user_data();
        let not_enough_app_data = |expected: usize| {
            verif_reporter
                .start_failure(
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
        let target_id_and_apid = TargetAndApidId::from_pus_tc(tc).unwrap();
        let active_request =
            ActivePusRequestStd::new(target_id_and_apid.into(), token, Duration::from_secs(30));
        let subservice_typed = Subservice::try_from(subservice);
        let invalid_subservice = || {
            // Invalid subservice
            verif_reporter
                .start_failure(
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

#[cfg(test)]
mod tests {
    use satrs::{
        mode::{ModeAndSubmode, ModeRequest},
        pus::mode::Subservice,
        spacepackets::{
            ecss::tc::{PusTcCreator, PusTcSecondaryHeader},
            SpHeader,
        },
    };

    use crate::pus::tests::{PusConverterTestbench, TEST_APID, TEST_APID_TARGET_ID};

    use super::ModeRequestConverter;

    #[test]
    fn mode_converter_read_mode_request() {
        let mut testbench = PusConverterTestbench::new(ModeRequestConverter::default());
        let mut sp_header = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(200, Subservice::TcReadMode as u8);
        let mut app_data: [u8; 4] = [0; 4];
        app_data[0..4].copy_from_slice(&TEST_APID_TARGET_ID.to_be_bytes());
        let tc = PusTcCreator::new(&mut sp_header, sec_header, &app_data, true);
        let token = testbench.add_tc(&tc);
        let (_active_req, req) = testbench
            .convert(token, &[], TEST_APID, TEST_APID_TARGET_ID)
            .expect("conversion has failed");
        assert_eq!(req, ModeRequest::ReadMode);
    }

    #[test]
    fn mode_converter_set_mode_request() {
        let mut testbench = PusConverterTestbench::new(ModeRequestConverter::default());
        let mut sp_header = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(200, Subservice::TcSetMode as u8);
        let mut app_data: [u8; 4 + ModeAndSubmode::RAW_LEN] = [0; 4 + ModeAndSubmode::RAW_LEN];
        let mode_and_submode = ModeAndSubmode::new(2, 1);
        app_data[0..4].copy_from_slice(&TEST_APID_TARGET_ID.to_be_bytes());
        mode_and_submode
            .write_to_be_bytes(&mut app_data[4..])
            .unwrap();
        let tc = PusTcCreator::new(&mut sp_header, sec_header, &app_data, true);
        let token = testbench.add_tc(&tc);
        let (_active_req, req) = testbench
            .convert(token, &[], TEST_APID, TEST_APID_TARGET_ID)
            .expect("conversion has failed");
        assert_eq!(req, ModeRequest::SetMode(mode_and_submode));
    }

    #[test]
    fn mode_converter_announce_mode() {
        let mut testbench = PusConverterTestbench::new(ModeRequestConverter::default());
        let mut sp_header = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
        let sec_header = PusTcSecondaryHeader::new_simple(200, Subservice::TcAnnounceMode as u8);
        let mut app_data: [u8; 4] = [0; 4];
        app_data[0..4].copy_from_slice(&TEST_APID_TARGET_ID.to_be_bytes());
        let tc = PusTcCreator::new(&mut sp_header, sec_header, &app_data, true);
        let token = testbench.add_tc(&tc);
        let (_active_req, req) = testbench
            .convert(token, &[], TEST_APID, TEST_APID_TARGET_ID)
            .expect("conversion has failed");
        assert_eq!(req, ModeRequest::AnnounceMode);
    }

    #[test]
    fn mode_converter_announce_mode_recursively() {
        let mut testbench = PusConverterTestbench::new(ModeRequestConverter::default());
        let mut sp_header = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
        let sec_header =
            PusTcSecondaryHeader::new_simple(200, Subservice::TcAnnounceModeRecursive as u8);
        let mut app_data: [u8; 4] = [0; 4];
        app_data[0..4].copy_from_slice(&TEST_APID_TARGET_ID.to_be_bytes());
        let tc = PusTcCreator::new(&mut sp_header, sec_header, &app_data, true);
        let token = testbench.add_tc(&tc);
        let (_active_req, req) = testbench
            .convert(token, &[], TEST_APID, TEST_APID_TARGET_ID)
            .expect("conversion has failed");
        assert_eq!(req, ModeRequest::AnnounceModeRecursive);
    }

    // TODO: Add reply handler tests.
}
