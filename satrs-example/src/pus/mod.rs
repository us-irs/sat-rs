use crate::requests::GenericRequestRouter;
use crate::tmtc::MpscStoreAndSendError;
use log::warn;
use satrs::pus::verification::{
    self, FailParams, TcStateAccepted, VerificationReportingProvider, VerificationToken,
};
use satrs::pus::{
    ActiveRequestMapProvider, ActiveRequestProvider, EcssTcAndToken, EcssTcInMemConverter,
    EcssTcReceiverCore, EcssTmSenderCore, EcssTmtcError, GenericConversionError,
    GenericRoutingError, PusPacketHandlerResult, PusPacketHandlingError, PusReplyHandler,
    PusRequestRouter, PusServiceHelper, PusTcToRequestConverter, TcInMemory,
};
use satrs::queue::GenericReceiveError;
use satrs::request::GenericMessage;
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusServiceId;
use satrs::spacepackets::time::cds::TimeProvider;
use satrs::spacepackets::time::TimeWriter;
use satrs_example::config::{tmtc_err, CustomPusServiceId};
use std::fmt::Debug;
use std::sync::mpsc::{self, Sender};

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

/// This is a generic handler class for all PUS services where a PUS telecommand is converted
/// to a targeted request.
///
/// The generic steps for this process are the following
///
///  1. Poll for TC packets
///  2. Convert the raw packets to a [PusTcReader].
///  3. Convert the PUS TC to a typed request using the [PusTcToRequestConverter].
///  4. Route the requests using the [GenericRequestRouter].
///  5. Add the request to the active request map using the [ActiveRequestMapProvider] abstraction.
///  6. Check for replies which complete the forwarded request. The handler takes care of
///     the verification process.
///  7. Check for timeouts of active requests. Generally, the timeout on the service level should
///     be highest expected timeout for the given target.
///
/// The handler exposes the following API:
///
///  1. [Self::handle_one_tc] which tries to poll and handle one TC packet, covering steps 1-5.
///  2. [Self::check_one_reply] which tries to poll and handle one reply, covering step 6.
///  3. [Self::check_for_request_timeouts] which checks for request timeouts, covering step 7.
pub struct PusTargetedRequestService<
    TcReceiver: EcssTcReceiverCore,
    TmSender: EcssTmSenderCore,
    TcInMemConverter: EcssTcInMemConverter,
    VerificationReporter: VerificationReportingProvider,
    RequestConverter: PusTcToRequestConverter<ActiveRequestInfo, RequestType, Error = GenericConversionError>,
    ReplyHandler: PusReplyHandler<ActiveRequestInfo, ReplyType, Error = EcssTmtcError>,
    ActiveRequestMap: ActiveRequestMapProvider<ActiveRequestInfo>,
    ActiveRequestInfo: ActiveRequestProvider,
    RequestType,
    ReplyType,
> {
    pub service_helper:
        PusServiceHelper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
    pub request_router: GenericRequestRouter,
    pub request_converter: RequestConverter,
    pub active_request_map: ActiveRequestMap,
    pub reply_handler: ReplyHandler,
    pub reply_receiver: mpsc::Receiver<GenericMessage<ReplyType>>,
    phantom: std::marker::PhantomData<(RequestType, ActiveRequestInfo, ReplyType)>,
}

impl<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
        RequestConverter: PusTcToRequestConverter<ActiveRequestInfo, RequestType, Error = GenericConversionError>,
        ReplyHandler: PusReplyHandler<ActiveRequestInfo, ReplyType, Error = EcssTmtcError>,
        ActiveRequestMap: ActiveRequestMapProvider<ActiveRequestInfo>,
        ActiveRequestInfo: ActiveRequestProvider,
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
        ActiveRequestInfo,
        RequestType,
        ReplyType,
    >
where
    GenericRequestRouter: PusRequestRouter<RequestType, Error = GenericRoutingError>,
{
    pub fn new(
        service_helper: PusServiceHelper<
            TcReceiver,
            TmSender,
            TcInMemConverter,
            VerificationReporter,
        >,
        request_converter: RequestConverter,
        active_request_map: ActiveRequestMap,
        reply_hook: ReplyHandler,
        request_router: GenericRequestRouter,
        reply_receiver: mpsc::Receiver<GenericMessage<ReplyType>>,
    ) -> Self {
        Self {
            service_helper,
            request_converter,
            active_request_map,
            reply_handler: reply_hook,
            request_router,
            reply_receiver,
            phantom: std::marker::PhantomData,
        }
    }

    pub fn poll_and_handle_next_tc(
        &mut self,
        time_stamp: &[u8],
    ) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        let possible_packet = self.service_helper.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(PusPacketHandlerResult::Empty);
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        self.service_helper
            .tc_in_mem_converter_mut()
            .cache(&ecss_tc_and_token.tc_in_memory)?;
        let tc = self.service_helper.tc_in_mem_converter().convert()?;
        let (request_info, request) = match self.request_converter.convert(
            ecss_tc_and_token.token,
            &tc,
            time_stamp,
            &self.service_helper.common.verif_reporter,
        ) {
            Ok((info, req)) => (info, req),
            Err(e) => {
                self.handle_conversion_to_request_error(&e, ecss_tc_and_token.token, time_stamp);
                return Err(e.into());
            }
        };
        let verif_request_id = verification::RequestId::new(&tc);
        //if let Err(e) =
        match self
            .request_router
            .route(request_info.target_id(), request, request_info.token())
        {
            Ok(()) => {
                self.active_request_map
                    .insert(&verif_request_id.into(), request_info);
            }
            Err(e) => {
                self.request_router.handle_error_generic(
                    &request_info,
                    &tc,
                    e,
                    time_stamp,
                    self.service_helper.verif_reporter(),
                );
            }
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }

    fn handle_conversion_to_request_error(
        &mut self,
        error: &GenericConversionError,
        token: VerificationToken<TcStateAccepted>,
        time_stamp: &[u8],
    ) {
        match error {
            GenericConversionError::WrongService(service) => {
                let service_slice: [u8; 1] = [*service];
                self.service_helper
                    .verif_reporter()
                    .completion_failure(
                        token,
                        FailParams::new(time_stamp, &tmtc_err::INVALID_PUS_SERVICE, &service_slice),
                    )
                    .expect("Sending completion failure failed");
            }
            GenericConversionError::InvalidSubservice(subservice) => {
                let subservice_slice: [u8; 1] = [*subservice];
                self.service_helper
                    .verif_reporter()
                    .completion_failure(
                        token,
                        FailParams::new(
                            time_stamp,
                            &tmtc_err::INVALID_PUS_SUBSERVICE,
                            &subservice_slice,
                        ),
                    )
                    .expect("Sending completion failure failed");
            }
            GenericConversionError::NotEnoughAppData { expected, found } => {
                let mut context_info = (*found as u32).to_be_bytes().to_vec();
                context_info.extend_from_slice(&(*expected as u32).to_be_bytes());
                self.service_helper
                    .verif_reporter()
                    .completion_failure(
                        token,
                        FailParams::new(time_stamp, &tmtc_err::NOT_ENOUGH_APP_DATA, &context_info),
                    )
                    .expect("Sending completion failure failed");
            }
            // Do nothing.. this is service-level and can not be handled generically here.
            GenericConversionError::InvalidAppData(_) => (),
        }
    }

    pub fn poll_and_check_next_reply(&mut self, time_stamp: &[u8]) -> Result<bool, EcssTmtcError> {
        match self.reply_receiver.try_recv() {
            Ok(reply) => {
                self.handle_reply(&reply, time_stamp)?;
                Ok(true)
            }
            Err(e) => match e {
                mpsc::TryRecvError::Empty => Ok(false),
                mpsc::TryRecvError::Disconnected => Err(EcssTmtcError::Receive(
                    GenericReceiveError::TxDisconnected(None),
                )),
            },
        }
    }

    pub fn handle_reply(
        &mut self,
        reply: &GenericMessage<ReplyType>,
        time_stamp: &[u8],
    ) -> Result<(), EcssTmtcError> {
        let active_req_opt = self.active_request_map.get(reply.request_id);
        if active_req_opt.is_none() {
            self.reply_handler
                .handle_unexpected_reply(reply, &self.service_helper.common.tm_sender)?;
            return Ok(());
        }
        let active_request = active_req_opt.unwrap();
        let request_finished = self
            .reply_handler
            .handle_reply(
                reply,
                active_request,
                &self.service_helper.common.verif_reporter,
                time_stamp,
                &self.service_helper.common.tm_sender,
            )
            .unwrap_or(false);
        if request_finished {
            self.active_request_map.remove(reply.request_id);
        }
        Ok(())
    }

    pub fn check_for_request_timeouts(&mut self) {
        let mut requests_to_delete = Vec::new();
        self.active_request_map
            .for_each(|request_id, request_info| {
                if request_info.has_timed_out() {
                    requests_to_delete.push(*request_id);
                }
            });
        if !requests_to_delete.is_empty() {
            for request_id in requests_to_delete {
                self.active_request_map.remove(request_id);
            }
        }
    }
}

pub fn generic_pus_request_timeout_handler(
    active_request: &(impl ActiveRequestProvider + Debug),
    verification_handler: &impl VerificationReportingProvider,
    time_stamp: &[u8],
    service_str: &'static str,
) -> Result<(), EcssTmtcError> {
    log::warn!("timeout for active request {active_request:?} on {service_str} service");
    verification_handler
        .completion_failure(
            active_request.token(),
            FailParams::new(
                time_stamp,
                &satrs_example::config::tmtc_err::REQUEST_TIMEOUT,
                &[],
            ),
        )
        .map_err(|e| e.0)?;
    Ok(())
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
