use std::sync::mpsc;

use satrs::{
    pus::{
        verification::{
            TcStateAccepted, VerificationReporterCfg, VerificationReporterWithVecMpscSender,
            VerificationReportingProvider, VerificationToken,
        },
        ActivePusRequest, DefaultActiveRequestMap, EcssTcInVecConverter, PusRequestRouter,
        PusServiceReplyHandler, PusTargetedRequestHandler, PusTcToRequestConverter,
        ReplyHandlerHook, TmAsVecSenderWithId,
    },
    TargetId,
};
use spacepackets::{
    ecss::{tc::PusTcReader, PusPacket},
    CcsdsPacket,
};

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum DummyRequest {
    Ping,
    WithParam(u32),
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum DummyReply {
    Pong,
}

pub struct DummyRequestConverter {}

impl PusTcToRequestConverter<DummyRequest> for DummyRequestRouter {
    type Error = ();

    fn convert(
        &mut self,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) -> Result<(TargetId, DummyRequest), Self::Error> {
        if tc.service() == 205 && tc.subservice() == 1 {
            return Ok((tc.apid().into(), DummyRequest::Ping));
        }
        Err(())
    }
}

pub struct DummyRequestRouter {
    dummy_1_sender: mpsc::Sender<DummyRequest>,
    dummy_2_sender: mpsc::Sender<DummyRequest>,
}

impl PusRequestRouter<DummyRequest> for DummyRequestRouter {
    type Error = ();
    fn route(
        &self,
        target_id: TargetId,
        request: DummyRequest,
        token: VerificationToken<TcStateAccepted>,
    ) -> Result<(), Self::Error> {
        if target_id == DummyTargetId::Object1 as u64 {
            self.dummy_1_sender.send(request).ok();
        } else {
            self.dummy_2_sender.send(request).ok();
        }
        Ok(())
    }

    fn handle_error(
        &self,
        target_id: TargetId,
        token: VerificationToken<TcStateAccepted>,
        tc: &PusTcReader,
        error: Self::Error,
        time_stamp: &[u8],
        verif_reporter: &impl VerificationReportingProvider,
    ) {
        panic!("routing error");
    }
}

#[derive(Default)]
pub struct DummyReplyUserHook {}

impl ReplyHandlerHook<ActivePusRequest, DummyReply> for DummyReplyUserHook {
    fn handle_unexpected_reply(&mut self, reply: &satrs::request::GenericMessage<DummyReply>) {
        todo!()
    }

    fn timeout_callback(&self, active_request: &ActivePusRequest) {
        todo!()
    }

    fn timeout_error_code(&self) -> satrs_shared::res_code::ResultU16 {
        todo!()
    }
}

pub type PusDummyRequestHandler = PusTargetedRequestHandler<
    mpsc::Sender<Vec<u8>>,
    mpsc::Sender<Vec<u8>>,
    EcssTcInVecConverter,
    VerificationReporterWithVecMpscSender,
    DummyRequestConverter,
    DummyRequestRouter,
    DummyRequest,
>;
pub type PusDummyReplyHandler = PusServiceReplyHandler<
    VerificationReporterWithVecMpscSender,
    DefaultActiveRequestMap<ActivePusRequest>,
    DummyReplyUserHook,
    mpsc::Sender<Vec<u8>>,
    ActivePusRequest,
    DummyReply,
>;
const TEST_APID: u16 = 5;

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum DummyTargetId {
    Object1 = 1,
    Object2 = 2,
}

pub enum DummyChannelId {
    Router = 1,
    Object1 = 2,
    Object2 = 3,
}

fn main() {
    let reporter_cfg = VerificationReporterCfg::new(TEST_APID, 2, 2, 256).unwrap();
    let (tm_sender, tm_receiver) = mpsc::channel();
    let tm_sender_with_wrapper =
        TmAsVecSenderWithId::new(DummyChannelId::Router as u32, "ROUTER", tm_sender.clone());
    let verification_handler =
        VerificationReporterWithVecMpscSender::new(&reporter_cfg, tm_sender_with_wrapper);

    // let dummy_request_handler = PusDummyRequestHandler::new()
    let dummy_reply_handler = PusDummyReplyHandler::new_from_now(
        verification_handler,
        DefaultActiveRequestMap::default(),
        256,
        DummyReplyUserHook::default(),
        tm_sender.clone(),
    );
}
