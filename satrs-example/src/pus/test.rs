use log::{info, warn};
use satrs_core::events::EventU32;
use satrs_core::params::Params;
use satrs_core::pus::test::PusService17TestHandler;
use satrs_core::pus::verification::FailParams;
use satrs_core::pus::PusPacketHandlerResult;
use satrs_core::spacepackets::ecss::tc::PusTcReader;
use satrs_core::spacepackets::ecss::PusPacket;
use satrs_core::spacepackets::time::cds::TimeProvider;
use satrs_core::spacepackets::time::TimeWriter;
use satrs_example::{tmtc_err, TEST_EVENT};
use std::sync::mpsc::Sender;

pub struct Service17CustomWrapper {
    pub pus17_handler: PusService17TestHandler,
    pub test_srv_event_sender: Sender<(EventU32, Option<Params>)>,
}

impl Service17CustomWrapper {
    pub fn handle_next_packet(&mut self) -> bool {
        let res = self.pus17_handler.handle_one_tc();
        if res.is_err() {
            warn!("PUS17 handler failed with error {:?}", res.unwrap_err());
            return true;
        }
        match res.unwrap() {
            PusPacketHandlerResult::RequestHandled => {
                info!("Received PUS ping command TC[17,1]");
                info!("Sent ping reply PUS TM[17,2]");
            }
            PusPacketHandlerResult::RequestHandledPartialSuccess(partial_err) => {
                warn!(
                    "Handled PUS ping command with partial success: {:?}",
                    partial_err
                );
            }
            PusPacketHandlerResult::SubserviceNotImplemented(subservice, _) => {
                warn!("PUS17: Subservice {subservice} not implemented")
            }
            PusPacketHandlerResult::CustomSubservice(subservice, token) => {
                let psb_mut = &mut self.pus17_handler.psb;
                let buf = psb_mut.pus_buf;
                let (tc, _) = PusTcReader::new(&buf).unwrap();
                let time_stamper = TimeProvider::from_now_with_u16_days().unwrap();
                let mut stamp_buf: [u8; 7] = [0; 7];
                time_stamper.write_to_bytes(&mut stamp_buf).unwrap();
                if subservice == 128 {
                    info!("Generating test event");
                    self.test_srv_event_sender
                        .send((TEST_EVENT.into(), None))
                        .expect("Sending test event failed");
                    let start_token = psb_mut
                        .verification_handler
                        .get_mut()
                        .start_success(token.into(), Some(&stamp_buf))
                        .expect("Error sending start success");
                    psb_mut
                        .verification_handler
                        .get_mut()
                        .completion_success(start_token, Some(&stamp_buf))
                        .expect("Error sending completion success");
                } else {
                    let fail_data = [tc.subservice()];
                    self.pus17_handler
                        .psb
                        .verification_handler
                        .get_mut()
                        .start_failure(
                            token,
                            FailParams::new(
                                Some(&stamp_buf),
                                &tmtc_err::INVALID_PUS_SUBSERVICE,
                                Some(&fail_data),
                            ),
                        )
                        .expect("Sending start failure verification failed");
                }
            }
            PusPacketHandlerResult::Empty => {
                return true;
            }
        }
        false
    }
}
