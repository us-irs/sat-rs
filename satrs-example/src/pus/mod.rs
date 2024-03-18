use crate::requests::GenericRequestRouter;
use crate::tmtc::MpscStoreAndSendError;
use log::warn;
use satrs::pus::verification::{
    self, FailParams, VerificationReporter, VerificationReporterWithSharedPoolMpscBoundedSender,
    VerificationReporterWithVecMpscSender, VerificationReportingProvider,
};
use satrs::pus::{
    ActiveRequestMapProvider, ActiveRequestProvider, DefaultActiveRequestMap, EcssTcAndToken,
    EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter, EcssTcReceiverCore,
    EcssTmSenderCore, GenericRoutingError, MpscTcReceiver, PusPacketHandlerResult, PusReplyHandler,
    PusServiceHelper, PusServiceReplyHandler, PusTcToRequestConverter, ReplyHandlerHook,
    TcInMemory, TmAsVecSenderWithMpsc, TmInSharedPoolSenderWithBoundedMpsc,
};
use satrs::request::GenericMessage;
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusServiceId;
use satrs::spacepackets::time::cds::TimeProvider;
use satrs::spacepackets::time::TimeWriter;
use satrs_example::config::{tmtc_err, CustomPusServiceId};
use std::sync::mpsc::Sender;

pub mod action;
pub mod event;
pub mod hk;
pub mod scheduler;
pub mod stack;
pub mod test;

pub struct PusTcMpscRouter {
    pub test_service_receiver: Sender<EcssTcAndToken>,
    pub event_service_receiver: Sender<EcssTcAndToken>,
    pub sched_service_receiver: Sender<EcssTcAndToken>,
    pub hk_service_receiver: Sender<EcssTcAndToken>,
    pub action_service_receiver: Sender<EcssTcAndToken>,
}

pub struct PusReceiver<VerificationReporter: VerificationReportingProvider> {
    pub verif_reporter: VerificationReporter,
    pub pus_router: PusTcMpscRouter,
    stamp_helper: TimeStampHelper,
}

struct TimeStampHelper {
    stamper: TimeProvider,
    time_stamp: [u8; 7],
}

impl TimeStampHelper {
    pub fn stamp(&self) -> &[u8] {
        &self.time_stamp
    }

    pub fn update_from_now(&mut self) {
        self.stamper
            .update_from_now()
            .expect("Updating timestamp failed");
        self.stamper
            .write_to_bytes(&mut self.time_stamp)
            .expect("Writing timestamp failed");
    }
}

impl Default for TimeStampHelper {
    fn default() -> Self {
        Self {
            stamper: TimeProvider::from_now_with_u16_days().expect("creating time stamper failed"),
            time_stamp: Default::default(),
        }
    }
}

impl<VerificationReporter: VerificationReportingProvider> PusReceiver<VerificationReporter> {
    pub fn new(verif_reporter: VerificationReporter, pus_router: PusTcMpscRouter) -> Self {
        Self {
            verif_reporter,
            pus_router,
            stamp_helper: TimeStampHelper::default(),
        }
    }
}

pub struct PusTargetedRequestService<
    TcReceiver: EcssTcReceiverCore,
    TmSender: EcssTmSenderCore,
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
    RequestConverter: PusTcToRequestConverter<RequestType>,
    ReplyHook: ReplyHandlerHook<RequestType, ReplyType>,
    ActiveRequestMap: ActiveRequestMapProvider<RequestType>,
    ActiveRequestType: ActiveRequestProvider,
    RequestType,
    ReplyType,
> {
    pub service_helper:
        PusServiceHelper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    pub request_router: GenericRequestRouter,
    pub request_converter: RequestConverter,
    pub active_request_map: ActiveRequestMap,
    pub reply_hook: ReplyHook,
    phantom: std::marker::PhantomData<RequestType>,
}

impl<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
        RequestConverter: PusTcToRequestConverter<RequestType>,
        ReplyHandler: PusReplyHandler<ActiveRequestType, ReplyType>,
        ActiveRequestMap: ActiveRequestMapProvider<ActiveRequestType>,
        ActiveRequestType: ActiveRequestProvider,
        RequestType,
        ReplyType,
    >
    PusTargetedRequestService<
        TcReceiver,
        TmSender,
        TcInMemConverter,
        VerificationReporter,
        RequestConverter,
        ReplyHandler,
        ActiveRequestMap,
        ActiveRequestType,
        RequestType,
        ReplyType,
    >
{
    pub fn new(
        service_helper: PusServiceHelper<
            TcReceiver,
            TmSender,
            TcInMemConverter,
            VerificationReporter,
        >,
        request_converter: RequestConverter,
        request_router: GenericRequestRouter,
        active_request_map: ActiveRequestMap,
        reply_hook: ReplyHandler,
    ) -> Self {
        Self {
            service_helper,
            request_router,
            request_converter,
            active_request_map,
            reply_hook,
            phantom: std::marker::PhantomData,
        }
    }

    pub fn handle_one_tc(&mut self) {
        let possible_packet = self.service_helper.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(PusPacketHandlerResult::Empty);
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        let tc = self
            .service_helper
            .tc_in_mem_converter
            .convert_ecss_tc_in_memory_to_reader(&ecss_tc_and_token.tc_in_memory)?;
        let mut partial_error = None;
        let time_stamp = get_current_cds_short_timestamp(&mut partial_error);
        let (active_request, action_request) = self.request_converter.convert(
            ecss_tc_and_token.token,
            &tc,
            &time_stamp,
            &self.service_helper.common.verification_handler,
        )?;
        if let Err(e) = self.request_router.route(
            active_request.target_id(),
            action_request,
            ecss_tc_and_token.token,
        ) {
            let verif_request_id = verification::RequestId::new(&tc);
            self.active_request_map
                .insert(verif_request_id.into(), active_request);
            self.request_router.handle_error(
                target_id,
                ecss_tc_and_token.token,
                &tc,
                e.clone(),
                &time_stamp,
                &self.service_helper.common.verification_handler,
            );
            return Err(e.into());
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }

    pub fn insert_reply(&mut self, reply: &GenericMessage<ReplyType>) {
        // self.reply_hook.insert_reply(reply, &self.active_request_map);
    }
}

impl<VerificationReporter: VerificationReportingProvider> PusReceiver<VerificationReporter> {
    pub fn handle_tc_packet(
        &mut self,
        tc_in_memory: TcInMemory,
        service: u8,
        pus_tc: &PusTcReader,
    ) -> Result<PusPacketHandlerResult, MpscStoreAndSendError> {
        let init_token = self.verif_reporter.add_tc(pus_tc);
        self.stamp_helper.update_from_now();
        let accepted_token = self
            .verif_reporter
            .acceptance_success(init_token, self.stamp_helper.stamp())
            .expect("Acceptance success failure");
        let service = PusServiceId::try_from(service);
        match service {
            Ok(standard_service) => match standard_service {
                PusServiceId::Test => {
                    self.pus_router.test_service_receiver.send(EcssTcAndToken {
                        tc_in_memory,
                        token: Some(accepted_token.into()),
                    })?
                }
                PusServiceId::Housekeeping => {
                    self.pus_router.hk_service_receiver.send(EcssTcAndToken {
                        tc_in_memory,
                        token: Some(accepted_token.into()),
                    })?
                }
                PusServiceId::Event => {
                    self.pus_router
                        .event_service_receiver
                        .send(EcssTcAndToken {
                            tc_in_memory,
                            token: Some(accepted_token.into()),
                        })?
                }
                PusServiceId::Scheduling => {
                    self.pus_router
                        .sched_service_receiver
                        .send(EcssTcAndToken {
                            tc_in_memory,
                            token: Some(accepted_token.into()),
                        })?
                }
                _ => {
                    let result = self.verif_reporter.start_failure(
                        accepted_token,
                        FailParams::new(
                            self.stamp_helper.stamp(),
                            &tmtc_err::PUS_SERVICE_NOT_IMPLEMENTED,
                            &[standard_service as u8],
                        ),
                    );
                    if result.is_err() {
                        warn!("Sending verification failure failed");
                    }
                }
            },
            Err(e) => {
                if let Ok(custom_service) = CustomPusServiceId::try_from(e.number) {
                    match custom_service {
                        CustomPusServiceId::Mode => {
                            // TODO: Fix mode service.
                            //self.handle_mode_service(pus_tc, accepted_token)
                        }
                        CustomPusServiceId::Health => {}
                    }
                } else {
                    self.verif_reporter
                        .start_failure(
                            accepted_token,
                            FailParams::new(
                                self.stamp_helper.stamp(),
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                &[e.number],
                            ),
                        )
                        .expect("Start failure verification failed")
                }
            }
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}
