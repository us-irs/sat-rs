use crate::tmtc::MpscStoreAndSendError;
use satrs_core::events::EventU32;
use satrs_core::params::Params;
use satrs_core::pool::StoreAddr;
use satrs_core::pus::verification::{FailParams, StdVerifReporterWithSender};
use satrs_core::pus::AcceptedTc;
use satrs_core::seq_count::SeqCountProviderSyncClonable;
use satrs_core::spacepackets::ecss::PusServiceId;
use satrs_core::spacepackets::tc::PusTc;
use satrs_core::spacepackets::time::cds::TimeProvider;
use satrs_core::spacepackets::time::TimeWriter;
use satrs_core::tmtc::tm_helper::PusTmWithCdsShortHelper;
use satrs_example::{tmtc_err, CustomPusServiceId};
use std::sync::mpsc::Sender;

pub mod event;
pub mod scheduler;
pub mod test;

pub struct PusTcMpscRouter {
    pub test_service_receiver: Sender<AcceptedTc>,
    pub event_service_receiver: Sender<AcceptedTc>,
    pub sched_service_receiver: Sender<AcceptedTc>,
    pub hk_service_receiver: Sender<AcceptedTc>,
    pub action_service_receiver: Sender<AcceptedTc>,
}

pub struct PusReceiver {
    pub tm_helper: PusTmWithCdsShortHelper,
    pub tm_args: PusTmArgs,
    pub tc_args: PusTcArgs,
    stamp_helper: TimeStampHelper,
}

pub struct PusTmArgs {
    /// All verification reporting is done with this reporter.
    pub verif_reporter: StdVerifReporterWithSender,
    /// Sequence count provider for TMs sent from within pus demultiplexer
    pub seq_count_provider: SeqCountProviderSyncClonable,
}

impl PusTmArgs {
    fn vr(&mut self) -> &mut StdVerifReporterWithSender {
        &mut self.verif_reporter
    }
}

pub struct PusTcArgs {
    /// This routes all telecommands to their respective recipients
    pub pus_router: PusTcMpscRouter,
    /// Used to send events from within the TC router
    pub event_sender: Sender<(EventU32, Option<Params>)>,
}

struct TimeStampHelper {
    stamper: TimeProvider,
    time_stamp: [u8; 7],
}

impl TimeStampHelper {
    pub fn new() -> Self {
        Self {
            stamper: TimeProvider::new_with_u16_days(0, 0),
            time_stamp: [0; 7],
        }
    }

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

impl PusReceiver {
    pub fn new(apid: u16, tm_arguments: PusTmArgs, tc_arguments: PusTcArgs) -> Self {
        Self {
            tm_helper: PusTmWithCdsShortHelper::new(apid),
            tm_args: tm_arguments,
            tc_args: tc_arguments,
            stamp_helper: TimeStampHelper::new(),
        }
    }
}

impl PusReceiver {
    pub fn handle_tc_packet(
        &mut self,
        store_addr: StoreAddr,
        service: u8,
        pus_tc: &PusTc,
    ) -> Result<(), MpscStoreAndSendError> {
        let init_token = self.tm_args.verif_reporter.add_tc(pus_tc);
        self.stamp_helper.update_from_now();
        let accepted_token = self
            .tm_args
            .vr()
            .acceptance_success(init_token, Some(self.stamp_helper.stamp()))
            .expect("Acceptance success failure");
        let service = PusServiceId::try_from(service);
        match service {
            Ok(standard_service) => match standard_service {
                PusServiceId::Test => {
                    let res = self
                        .tc_args
                        .pus_router
                        .test_service_receiver
                        .send((store_addr, accepted_token));
                    match res {
                        Ok(_) => {}
                        Err(e) => {
                            println!("Error {e}")
                        }
                    }
                }
                PusServiceId::Housekeeping => self
                    .tc_args
                    .pus_router
                    .hk_service_receiver
                    .send((store_addr, accepted_token))
                    .unwrap(),
                PusServiceId::Event => self
                    .tc_args
                    .pus_router
                    .event_service_receiver
                    .send((store_addr, accepted_token))
                    .unwrap(),
                PusServiceId::Scheduling => self
                    .tc_args
                    .pus_router
                    .sched_service_receiver
                    .send((store_addr, accepted_token))
                    .unwrap(),
                _ => self
                    .tm_args
                    .verif_reporter
                    .start_failure(
                        accepted_token,
                        FailParams::new(
                            Some(self.stamp_helper.stamp()),
                            &tmtc_err::PUS_SERVICE_NOT_IMPLEMENTED,
                            Some(&[standard_service as u8]),
                        ),
                    )
                    .expect("Start failure verification failed"),
            },
            Err(e) => {
                if let Ok(custom_service) = CustomPusServiceId::try_from(e.number) {
                    match custom_service {
                        CustomPusServiceId::Mode => {
                            //self.handle_mode_service(pus_tc, accepted_token)
                        }
                        CustomPusServiceId::Health => {}
                    }
                } else {
                    self.tm_args
                        .verif_reporter
                        .start_failure(
                            accepted_token,
                            FailParams::new(
                                Some(self.stamp_helper.stamp()),
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                Some(&[e.number]),
                            ),
                        )
                        .expect("Start failure verification failed")
                }
            }
        }
        Ok(())
    }
}
// impl PusServiceProvider for PusReceiver {
//     type Error = ();
//
//     fn handle_pus_tc_packet(
//         &mut self,
//         service: u8,
//         _header: &SpHeader,
//         pus_tc: &PusTc,
//     ) -> Result<(), Self::Error> {
//         let init_token = self.tm_args.verif_reporter.add_tc(pus_tc);
//         self.stamp_helper.update_from_now();
//         let accepted_token = self
//             .tm_args
//             .vr()
//             .acceptance_success(init_token, Some(self.stamp_helper.stamp()))
//             .expect("Acceptance success failure");
//         let service = PusServiceId::try_from(service);
//         match service {
//             Ok(standard_service) => match standard_service {
//                 PusServiceId::Test => self
//                     .tc_args
//                     .pus_router
//                     .test_service_receiver
//                     .send_tc(*pus_tc),
//                 PusServiceId::Housekeeping => {
//                     self.tc_args.pus_router.hk_service_receiver.send_tc(*pus_tc)
//                 } //self.handle_hk_request(pus_tc, accepted_token),
//                 PusServiceId::Event => self
//                     .tc_args
//                     .pus_router
//                     .event_service_receiver
//                     .send_tc(*pus_tc), //self.handle_event_request(pus_tc, accepted_token),
//                 PusServiceId::Scheduling => self
//                     .tc_args
//                     .pus_router
//                     .sched_service_receiver
//                     .send_tc(*pus_tc), //self.handle_scheduled_tc(pus_tc, accepted_token),
//                 _ => self
//                     .tm_args
//                     .verif_reporter
//                     .start_failure(
//                         accepted_token,
//                         FailParams::new(
//                             Some(self.stamp_helper.stamp()),
//                             &tmtc_err::PUS_SERVICE_NOT_IMPLEMENTED,
//                             Some(&[standard_service as u8]),
//                         ),
//                     )
//                     .expect("Start failure verification failed"),
//             },
//             Err(e) => {
//                 if let Ok(custom_service) = CustomPusServiceId::try_from(e.number) {
//                     match custom_service {
//                         CustomPusServiceId::Mode => {
//                             self.handle_mode_service(pus_tc, accepted_token)
//                         }
//                         CustomPusServiceId::Health => {}
//                     }
//                 } else {
//                     self.tm_args
//                         .verif_reporter
//                         .start_failure(
//                             accepted_token,
//                             FailParams::new(
//                                 Some(self.stamp_helper.stamp()),
//                                 &tmtc_err::INVALID_PUS_SUBSERVICE,
//                                 Some(&[e.number]),
//                             ),
//                         )
//                         .expect("Start failure verification failed")
//                 }
//             }
//         }
//         Ok(())
//     }
// }

// impl PusReceiver {
//
//     fn handle_hk_request(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
//         if pus_tc.user_data().is_none() {
//             self.tm_args
//                 .verif_reporter
//                 .start_failure(
//                     token,
//                     FailParams::new(
//                         Some(self.stamp_helper.stamp()),
//                         &tmtc_err::NOT_ENOUGH_APP_DATA,
//                         None,
//                     ),
//                 )
//                 .expect("Sending start failure TM failed");
//             return;
//         }
//         let user_data = pus_tc.user_data().unwrap();
//         if user_data.len() < 8 {
//             let err = if user_data.len() < 4 {
//                 &hk_err::TARGET_ID_MISSING
//             } else {
//                 &hk_err::UNIQUE_ID_MISSING
//             };
//             self.tm_args
//                 .verif_reporter
//                 .start_failure(
//                     token,
//                     FailParams::new(Some(self.stamp_helper.stamp()), err, None),
//                 )
//                 .expect("Sending start failure TM failed");
//             return;
//         }
//         let addressable_id = AddressableId::from_raw_be(user_data).unwrap();
//         if !self
//             .tc_args
//             .request_map
//             .contains_key(&addressable_id.target_id)
//         {
//             self.tm_args
//                 .verif_reporter
//                 .start_failure(
//                     token,
//                     FailParams::new(
//                         Some(self.stamp_helper.stamp()),
//                         &hk_err::UNKNOWN_TARGET_ID,
//                         None,
//                     ),
//                 )
//                 .expect("Sending start failure TM failed");
//             return;
//         }
//         let send_request = |target: TargetId, request: HkRequest| {
//             let sender = self
//                 .tc_args
//                 .request_map
//                 .get(&addressable_id.target_id)
//                 .unwrap();
//             sender
//                 .send(RequestWithToken::new(
//                     target,
//                     Request::HkRequest(request),
//                     token,
//                 ))
//                 .unwrap_or_else(|_| panic!("Sending HK request {request:?} failed"));
//         };
//         if PusPacket::subservice(pus_tc) == hk::Subservice::TcEnableHkGeneration as u8 {
//             send_request(
//                 addressable_id.target_id,
//                 HkRequest::Enable(addressable_id.unique_id),
//             );
//         } else if PusPacket::subservice(pus_tc) == hk::Subservice::TcDisableHkGeneration as u8 {
//             send_request(
//                 addressable_id.target_id,
//                 HkRequest::Disable(addressable_id.unique_id),
//             );
//         } else if PusPacket::subservice(pus_tc) == hk::Subservice::TcGenerateOneShotHk as u8 {
//             send_request(
//                 addressable_id.target_id,
//                 HkRequest::OneShot(addressable_id.unique_id),
//             );
//         } else if PusPacket::subservice(pus_tc)
//             == hk::Subservice::TcModifyHkCollectionInterval as u8
//         {
//             if user_data.len() < 12 {
//                 self.tm_args
//                     .verif_reporter
//                     .start_failure(
//                         token,
//                         FailParams::new(
//                             Some(self.stamp_helper.stamp()),
//                             &hk_err::COLLECTION_INTERVAL_MISSING,
//                             None,
//                         ),
//                     )
//                     .expect("Sending start failure TM failed");
//                 return;
//             }
//             send_request(
//                 addressable_id.target_id,
//                 HkRequest::ModifyCollectionInterval(
//                     addressable_id.unique_id,
//                     CollectionIntervalFactor::from_be_bytes(user_data[8..12].try_into().unwrap()),
//                 ),
//             );
//         }
//     }
//
//
//     fn handle_mode_service(&mut self, pus_tc: &PusTc, token: VerificationToken<TcStateAccepted>) {
//         let mut app_data_len = 0;
//         let app_data = pus_tc.user_data();
//         if app_data.is_some() {
//             app_data_len = pus_tc.user_data().unwrap().len();
//         }
//         if app_data_len < 4 {
//             self.tm_args
//                 .verif_reporter
//                 .start_failure(
//                     token,
//                     FailParams::new(
//                         Some(self.stamp_helper.stamp()),
//                         &tmtc_err::NOT_ENOUGH_APP_DATA,
//                         Some(format!("expected {} bytes, found {}", 4, app_data_len).as_bytes()),
//                     ),
//                 )
//                 .expect("Sending start failure TM failed");
//         }
//         let app_data = app_data.unwrap();
//         let mut invalid_subservice_handler = || {
//             self.tm_args
//                 .verif_reporter
//                 .start_failure(
//                     token,
//                     FailParams::new(
//                         Some(self.stamp_helper.stamp()),
//                         &tmtc_err::INVALID_PUS_SUBSERVICE,
//                         Some(&[PusPacket::subservice(pus_tc)]),
//                     ),
//                 )
//                 .expect("Sending start failure TM failed");
//         };
//         let subservice = mode::Subservice::try_from(PusPacket::subservice(pus_tc));
//         if let Ok(subservice) = subservice {
//             let forward_mode_request = |target_id, mode_request: ModeRequest| match self
//                 .tc_args
//                 .request_map
//                 .get(&target_id)
//             {
//                 None => warn!("not mode request recipient for target ID {target_id} found"),
//                 Some(sender_to_recipient) => {
//                     sender_to_recipient
//                         .send(RequestWithToken::new(
//                             target_id,
//                             Request::ModeRequest(mode_request),
//                             token,
//                         ))
//                         .expect("sending mode request failed");
//                 }
//             };
//             let mut valid_subservice = true;
//             match subservice {
//                 Subservice::TcSetMode => {
//                     let target_id = u32::from_be_bytes(app_data[0..4].try_into().unwrap());
//                     let min_len = ModeAndSubmode::raw_len() + 4;
//                     if app_data_len < min_len {
//                         self.tm_args
//                             .verif_reporter
//                             .start_failure(
//                                 token,
//                                 FailParams::new(
//                                     Some(self.stamp_helper.stamp()),
//                                     &tmtc_err::NOT_ENOUGH_APP_DATA,
//                                     Some(
//                                         format!("expected {min_len} bytes, found {app_data_len}")
//                                             .as_bytes(),
//                                     ),
//                                 ),
//                             )
//                             .expect("Sending start failure TM failed");
//                     }
//                     // Should never fail after size check
//                     let mode_submode = ModeAndSubmode::from_be_bytes(
//                         app_data[4..4 + ModeAndSubmode::raw_len()]
//                             .try_into()
//                             .unwrap(),
//                     )
//                     .unwrap();
//                     forward_mode_request(target_id, ModeRequest::SetMode(mode_submode));
//                 }
//                 Subservice::TcReadMode => {
//                     let target_id = u32::from_be_bytes(app_data[0..4].try_into().unwrap());
//                     forward_mode_request(target_id, ModeRequest::ReadMode);
//                 }
//                 Subservice::TcAnnounceMode => {
//                     let target_id = u32::from_be_bytes(app_data[0..4].try_into().unwrap());
//                     forward_mode_request(target_id, ModeRequest::AnnounceMode);
//                 }
//                 Subservice::TcAnnounceModeRecursive => {
//                     let target_id = u32::from_be_bytes(app_data[0..4].try_into().unwrap());
//                     forward_mode_request(target_id, ModeRequest::AnnounceModeRecursive);
//                 }
//                 _ => {
//                     warn!("Can not process mode request with subservice {subservice:?}");
//                     invalid_subservice_handler();
//                     valid_subservice = false;
//                 }
//             }
//             if valid_subservice {
//                 self.tm_args
//                     .verif_reporter
//                     .start_success(token, Some(self.stamp_helper.stamp()))
//                     .expect("sending start success TM failed");
//             }
//         } else {
//             invalid_subservice_handler();
//         }
//     }
// }
