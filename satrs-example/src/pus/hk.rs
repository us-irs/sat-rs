use crate::requests::{Request, RequestWithToken};
use log::{error, warn};
use satrs_core::hk::{CollectionIntervalFactor, HkRequest};
use satrs_core::pool::{SharedStaticMemoryPool, StoreAddr};
use satrs_core::pus::verification::{
    FailParams, StdVerifReporterWithSender, VerificationReporterWithSender,
};
use satrs_core::pus::{
    EcssTcAndToken, EcssTcInMemConverter, EcssTcInSharedStoreConverter, EcssTcInVecConverter,
    EcssTcReceiver, EcssTmSender, MpscTcReceiver, MpscTmAsVecSender, MpscTmInSharedPoolSender,
    PusPacketHandlerResult, PusPacketHandlingError, PusServiceBase, PusServiceHelper,
};
use satrs_core::spacepackets::ecss::{hk, PusPacket};
use satrs_core::tmtc::tm_helper::SharedTmPool;
use satrs_core::ChannelId;
use satrs_example::config::{hk_err, tmtc_err, TcReceiverId, TmSenderId, PUS_APID};
use satrs_example::TargetIdWithApid;
use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};

pub fn create_hk_service_static(
    shared_tm_store: SharedTmPool,
    tm_funnel_tx: mpsc::Sender<StoreAddr>,
    verif_reporter: VerificationReporterWithSender,
    tc_pool: SharedStaticMemoryPool,
    pus_hk_rx: mpsc::Receiver<EcssTcAndToken>,
    request_map: HashMap<TargetIdWithApid, mpsc::Sender<RequestWithToken>>,
) -> Pus3Wrapper<EcssTcInSharedStoreConverter> {
    let hk_srv_tm_sender = MpscTmInSharedPoolSender::new(
        TmSenderId::PusHk as ChannelId,
        "PUS_3_TM_SENDER",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    );
    let hk_srv_receiver =
        MpscTcReceiver::new(TcReceiverId::PusHk as ChannelId, "PUS_8_TC_RECV", pus_hk_rx);
    let pus_3_handler = PusService3HkHandler::new(
        Box::new(hk_srv_receiver),
        Box::new(hk_srv_tm_sender),
        PUS_APID,
        verif_reporter.clone(),
        EcssTcInSharedStoreConverter::new(tc_pool, 2048),
        request_map,
    );
    Pus3Wrapper { pus_3_handler }
}

pub fn create_hk_service_dynamic(
    tm_funnel_tx: mpsc::Sender<Vec<u8>>,
    verif_reporter: VerificationReporterWithSender,
    pus_hk_rx: mpsc::Receiver<EcssTcAndToken>,
    request_map: HashMap<TargetIdWithApid, mpsc::Sender<RequestWithToken>>,
) -> Pus3Wrapper<EcssTcInVecConverter> {
    let hk_srv_tm_sender = MpscTmAsVecSender::new(
        TmSenderId::PusHk as ChannelId,
        "PUS_3_TM_SENDER",
        tm_funnel_tx.clone(),
    );
    let hk_srv_receiver =
        MpscTcReceiver::new(TcReceiverId::PusHk as ChannelId, "PUS_8_TC_RECV", pus_hk_rx);
    let pus_3_handler = PusService3HkHandler::new(
        Box::new(hk_srv_receiver),
        Box::new(hk_srv_tm_sender),
        PUS_APID,
        verif_reporter.clone(),
        EcssTcInVecConverter::default(),
        request_map,
    );
    Pus3Wrapper { pus_3_handler }
}

pub struct PusService3HkHandler<TcInMemConverter: EcssTcInMemConverter> {
    psb: PusServiceHelper<TcInMemConverter>,
    request_handlers: HashMap<TargetIdWithApid, Sender<RequestWithToken>>,
}

impl<TcInMemConverter: EcssTcInMemConverter> PusService3HkHandler<TcInMemConverter> {
    pub fn new(
        tc_receiver: Box<dyn EcssTcReceiver>,
        tm_sender: Box<dyn EcssTmSender>,
        tm_apid: u16,
        verification_handler: StdVerifReporterWithSender,
        tc_in_mem_converter: TcInMemConverter,
        request_handlers: HashMap<TargetIdWithApid, Sender<RequestWithToken>>,
    ) -> Self {
        Self {
            psb: PusServiceHelper::new(
                tc_receiver,
                tm_sender,
                tm_apid,
                verification_handler,
                tc_in_mem_converter,
            ),
            request_handlers,
        }
    }

    fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
        let possible_packet = self.psb.retrieve_and_accept_next_packet()?;
        if possible_packet.is_none() {
            return Ok(PusPacketHandlerResult::Empty);
        }
        let ecss_tc_and_token = possible_packet.unwrap();
        let tc = self
            .psb
            .tc_in_mem_converter
            .convert_ecss_tc_in_memory_to_reader(&ecss_tc_and_token.tc_in_memory)?;
        let subservice = tc.subservice();
        let mut partial_error = None;
        let time_stamp = PusServiceBase::get_current_timestamp(&mut partial_error);
        let user_data = tc.user_data();
        if user_data.is_empty() {
            self.psb
                .common
                .verification_handler
                .borrow_mut()
                .start_failure(
                    ecss_tc_and_token.token,
                    FailParams::new(Some(&time_stamp), &tmtc_err::NOT_ENOUGH_APP_DATA, None),
                )
                .expect("Sending start failure TM failed");
            return Err(PusPacketHandlingError::NotEnoughAppData(
                "Expected at least 8 bytes of app data".into(),
            ));
        }
        if user_data.len() < 8 {
            let err = if user_data.len() < 4 {
                &hk_err::TARGET_ID_MISSING
            } else {
                &hk_err::UNIQUE_ID_MISSING
            };
            self.psb
                .common
                .verification_handler
                .borrow_mut()
                .start_failure(
                    ecss_tc_and_token.token,
                    FailParams::new(Some(&time_stamp), err, None),
                )
                .expect("Sending start failure TM failed");
            return Err(PusPacketHandlingError::NotEnoughAppData(
                "Expected at least 8 bytes of app data".into(),
            ));
        }
        let target_id = TargetIdWithApid::from_tc(&tc).expect("invalid tc format");
        let unique_id = u32::from_be_bytes(tc.user_data()[0..4].try_into().unwrap());
        if !self.request_handlers.contains_key(&target_id) {
            self.psb
                .common
                .verification_handler
                .borrow_mut()
                .start_failure(
                    ecss_tc_and_token.token,
                    FailParams::new(Some(&time_stamp), &hk_err::UNKNOWN_TARGET_ID, None),
                )
                .expect("Sending start failure TM failed");
            return Err(PusPacketHandlingError::NotEnoughAppData(format!(
                "Unknown target ID {target_id}"
            )));
        }
        let send_request = |target: TargetIdWithApid, request: HkRequest| {
            let sender = self.request_handlers.get(&target).unwrap();
            sender
                .send(RequestWithToken::new(
                    target,
                    Request::Hk(request),
                    ecss_tc_and_token.token,
                ))
                .unwrap_or_else(|_| panic!("Sending HK request {request:?} failed"));
        };
        if subservice == hk::Subservice::TcEnableHkGeneration as u8 {
            send_request(target_id, HkRequest::Enable(unique_id));
        } else if subservice == hk::Subservice::TcDisableHkGeneration as u8 {
            send_request(target_id, HkRequest::Disable(unique_id));
        } else if subservice == hk::Subservice::TcGenerateOneShotHk as u8 {
            send_request(target_id, HkRequest::OneShot(unique_id));
        } else if subservice == hk::Subservice::TcModifyHkCollectionInterval as u8 {
            if user_data.len() < 12 {
                self.psb
                    .common
                    .verification_handler
                    .borrow_mut()
                    .start_failure(
                        ecss_tc_and_token.token,
                        FailParams::new(
                            Some(&time_stamp),
                            &hk_err::COLLECTION_INTERVAL_MISSING,
                            None,
                        ),
                    )
                    .expect("Sending start failure TM failed");
                return Err(PusPacketHandlingError::NotEnoughAppData(
                    "Collection interval missing".into(),
                ));
            }
            send_request(
                target_id,
                HkRequest::ModifyCollectionInterval(
                    unique_id,
                    CollectionIntervalFactor::from_be_bytes(user_data[8..12].try_into().unwrap()),
                ),
            );
        }
        Ok(PusPacketHandlerResult::RequestHandled)
    }
}

pub struct Pus3Wrapper<TcInMemConverter: EcssTcInMemConverter> {
    pub(crate) pus_3_handler: PusService3HkHandler<TcInMemConverter>,
}

impl<TcInMemConverter: EcssTcInMemConverter> Pus3Wrapper<TcInMemConverter> {
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
