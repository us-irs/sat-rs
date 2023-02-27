mod ccsds;
mod hk;
mod logging;
mod pus;
mod requests;
mod tmtc;

use log::{info, warn};

use crate::hk::AcsHkIds;
use crate::logging::setup_logger;
use crate::requests::{Request, RequestWithToken};
use crate::tmtc::{
    core_tmtc_task, OtherArgs, PusTcSource, TcArgs, TcStore, TmArgs, TmFunnel, TmStore, PUS_APID,
};
use satrs_core::event_man::{
    EventManagerWithMpscQueue, MpscEventReceiver, MpscEventU32SendProvider, SendEventProvider,
};
use satrs_core::events::EventU32;
use satrs_core::hk::HkRequest;
use satrs_core::pool::{LocalPool, PoolCfg, StoreAddr};
use satrs_core::pus::event_man::{
    DefaultPusMgmtBackendProvider, EventReporter, EventRequest, EventRequestWithToken,
    PusEventDispatcher,
};
use satrs_core::pus::hk::Subservice as HkSubservice;
use satrs_core::pus::verification::{
    MpscVerifSender, VerificationReporterCfg, VerificationReporterWithSender,
};
use satrs_core::pus::{EcssTmErrorWithSend, EcssTmSenderCore};
use satrs_core::seq_count::SeqCountProviderSimple;
use satrs_core::spacepackets::{
    time::cds::TimeProvider,
    time::TimeWriter,
    tm::{PusTm, PusTmSecondaryHeader},
    SequenceFlags, SpHeader,
};
use satrs_core::tmtc::AddressableId;
use satrs_example::{RequestTargetId, OBSW_SERVER_ADDR, SERVER_PORT};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{channel, TryRecvError};
use std::sync::{mpsc, Arc, RwLock};
use std::thread;
use std::time::Duration;

#[derive(Clone)]
struct EventTmSender {
    store_helper: TmStore,
    sender: mpsc::Sender<StoreAddr>,
}

impl EventTmSender {
    fn new(store_helper: TmStore, sender: mpsc::Sender<StoreAddr>) -> Self {
        Self {
            store_helper,
            sender,
        }
    }
}

impl EcssTmSenderCore for EventTmSender {
    type Error = mpsc::SendError<StoreAddr>;

    fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmErrorWithSend<Self::Error>> {
        let addr = self.store_helper.add_pus_tm(&tm);
        self.sender
            .send(addr)
            .map_err(EcssTmErrorWithSend::SendError)
    }
}

fn main() {
    setup_logger().expect("setting up logging with fern failed");
    println!("Running OBSW example");
    let tm_pool = LocalPool::new(PoolCfg::new(vec![
        (30, 32),
        (15, 64),
        (15, 128),
        (15, 256),
        (15, 1024),
        (15, 2048),
    ]));
    let tm_store = TmStore {
        pool: Arc::new(RwLock::new(Box::new(tm_pool))),
    };
    let tc_pool = LocalPool::new(PoolCfg::new(vec![
        (30, 32),
        (15, 64),
        (15, 128),
        (15, 256),
        (15, 1024),
        (15, 2048),
    ]));
    let tc_store = TcStore {
        pool: Arc::new(RwLock::new(Box::new(tc_pool))),
    };

    let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let (tc_source_tx, tc_source_rx) = channel();
    let (tm_funnel_tx, tm_funnel_rx) = channel();
    let (tm_server_tx, tm_server_rx) = channel();
    let verif_sender = MpscVerifSender::new(tm_store.pool.clone(), tm_funnel_tx.clone());
    let verif_cfg = VerificationReporterCfg::new(
        PUS_APID,
        #[allow(clippy::box_default)]
        Box::new(SeqCountProviderSimple::default()),
        1,
        2,
        8,
    )
    .unwrap();
    // Every software component which needs to generate verification telemetry, gets a cloned
    // verification reporter.
    let verif_reporter = VerificationReporterWithSender::new(&verif_cfg, Box::new(verif_sender));
    let mut reporter_event_handler = verif_reporter.clone();
    let mut reporter_aocs = verif_reporter.clone();

    // Create event handling components
    // These sender handles are used to send event requests, for example to enable or disable
    // certain events
    let (event_request_tx, event_request_rx) = channel::<EventRequestWithToken>();
    // The sender handle is the primary sender handle for all components which want to create events.
    // The event manager will receive the RX handle to receive all the events.
    let (event_sender, event_man_rx) = channel();
    let event_recv = MpscEventReceiver::<EventU32>::new(event_man_rx);
    let mut event_man = EventManagerWithMpscQueue::new(Box::new(event_recv));

    // All events sent to the manager are routed to the PUS event manager, which generates PUS event
    // telemetry for each event.
    let event_reporter = EventReporter::new(PUS_APID, 128).unwrap();
    let pus_tm_backend = DefaultPusMgmtBackendProvider::<EventU32>::default();
    let mut pus_event_dispatcher =
        PusEventDispatcher::new(event_reporter, Box::new(pus_tm_backend));
    let (pus_event_man_tx, pus_event_man_rx) = channel();
    let pus_event_man_send_provider = MpscEventU32SendProvider::new(1, pus_event_man_tx);
    event_man.subscribe_all(pus_event_man_send_provider.id());
    event_man.add_sender(pus_event_man_send_provider);

    // Some request are targetable. This map is used to retrieve sender handles based on a target ID.
    let mut request_map = HashMap::new();
    let (acs_thread_tx, acs_thread_rx) = channel::<RequestWithToken>();
    request_map.insert(RequestTargetId::AcsSubsystem as u32, acs_thread_tx);

    let tc_source = PusTcSource {
        tc_store,
        tc_source: tc_source_tx,
    };

    // Create clones here to allow moving the values
    let core_args = OtherArgs {
        sock_addr,
        verif_reporter,
        event_sender,
        event_request_tx,
        request_map,
    };
    let tc_args = TcArgs {
        tc_source,
        tc_receiver: tc_source_rx,
    };
    let tm_args = TmArgs {
        tm_store: tm_store.clone(),
        tm_sink_sender: tm_funnel_tx.clone(),
        tm_server_rx,
    };

    let aocs_to_funnel = tm_funnel_tx.clone();
    let mut aocs_tm_store = tm_store.clone();

    info!("Starting TMTC task");
    let jh0 = thread::Builder::new()
        .name("TMTC".to_string())
        .spawn(move || {
            core_tmtc_task(core_args, tc_args, tm_args);
        })
        .unwrap();

    info!("Starting TM funnel task");
    let jh1 = thread::Builder::new()
        .name("TM Funnel".to_string())
        .spawn(move || {
            let tm_funnel = TmFunnel {
                tm_server_tx,
                tm_funnel_rx,
            };
            loop {
                if let Ok(addr) = tm_funnel.tm_funnel_rx.recv() {
                    tm_funnel
                        .tm_server_tx
                        .send(addr)
                        .expect("Sending TM to server failed");
                }
            }
        })
        .unwrap();

    info!("Starting event handling task");
    let jh2 = thread::Builder::new()
        .name("Event".to_string())
        .spawn(move || {
            let mut timestamp: [u8; 7] = [0; 7];
            let mut sender = EventTmSender::new(tm_store, tm_funnel_tx);
            let mut time_provider = TimeProvider::new_with_u16_days(0, 0);
            let mut report_completion = |event_req: EventRequestWithToken, timestamp: &[u8]| {
                reporter_event_handler
                    .completion_success(event_req.token, Some(timestamp))
                    .expect("Sending completion success failed");
            };
            loop {
                // handle event requests
                if let Ok(event_req) = event_request_rx.try_recv() {
                    match event_req.request {
                        EventRequest::Enable(event) => {
                            pus_event_dispatcher
                                .enable_tm_for_event(&event)
                                .expect("Enabling TM failed");
                            update_time(&mut time_provider, &mut timestamp);
                            report_completion(event_req, &timestamp);
                        }
                        EventRequest::Disable(event) => {
                            pus_event_dispatcher
                                .disable_tm_for_event(&event)
                                .expect("Disabling TM failed");
                            update_time(&mut time_provider, &mut timestamp);
                            report_completion(event_req, &timestamp);
                        }
                    }
                }

                // Perform the event routing.
                event_man
                    .try_event_handling()
                    .expect("event handling failed");

                // Perform the generation of PUS event packets
                if let Ok((event, _param)) = pus_event_man_rx.try_recv() {
                    update_time(&mut time_provider, &mut timestamp);
                    pus_event_dispatcher
                        .generate_pus_event_tm_generic(&mut sender, &timestamp, event, None)
                        .expect("Sending TM as event failed");
                }
                thread::sleep(Duration::from_millis(400));
            }
        })
        .unwrap();

    info!("Starting AOCS thread");
    let jh3 = thread::Builder::new()
        .name("AOCS".to_string())
        .spawn(move || {
            let mut timestamp: [u8; 7] = [0; 7];
            let mut time_provider = TimeProvider::new_with_u16_days(0, 0);
            loop {
                match acs_thread_rx.try_recv() {
                    Ok(request) => {
                        info!(
                            "ACS thread: Received HK request {:?}",
                            request.targeted_request
                        );
                        update_time(&mut time_provider, &mut timestamp);
                        match request.targeted_request.request {
                            Request::HkRequest(hk_req) => match hk_req {
                                HkRequest::OneShot(unique_id) => {
                                    let target = request.targeted_request.target_id;
                                    assert_eq!(target, RequestTargetId::AcsSubsystem as u32);
                                    if request.targeted_request.target_id
                                        == AcsHkIds::TestMgmSet as u32
                                    {
                                        let mut sp_header = SpHeader::tm(
                                            PUS_APID,
                                            SequenceFlags::Unsegmented,
                                            0,
                                            0,
                                        )
                                        .unwrap();
                                        let sec_header = PusTmSecondaryHeader::new_simple(
                                            3,
                                            HkSubservice::TmHkPacket as u8,
                                            &timestamp,
                                        );
                                        let mut buf: [u8; 8] = [0; 8];
                                        let addressable_id = AddressableId {
                                            target_id: target,
                                            unique_id,
                                        };
                                        addressable_id.write_to_be_bytes(&mut buf).unwrap();
                                        let pus_tm = PusTm::new(
                                            &mut sp_header,
                                            sec_header,
                                            Some(&buf),
                                            true,
                                        );
                                        let addr = aocs_tm_store.add_pus_tm(&pus_tm);
                                        aocs_to_funnel.send(addr).expect("Sending HK TM failed");
                                    }
                                }
                                HkRequest::Enable(_) => {}
                                HkRequest::Disable(_) => {}
                                HkRequest::ModifyCollectionInterval(_, _) => {}
                            },
                            Request::ModeRequest(_mode_req) => {
                                warn!("mode request handling not implemented yet")
                            }
                        }
                        let started_token = reporter_aocs
                            .start_success(request.token, Some(&timestamp))
                            .expect("Sending start success failed");
                        reporter_aocs
                            .completion_success(started_token, Some(&timestamp))
                            .expect("Sending completion success failed");
                    }
                    Err(e) => match e {
                        TryRecvError::Empty => {}
                        TryRecvError::Disconnected => {
                            warn!("ACS thread: Message Queue TX disconnected!")
                        }
                    },
                }
                thread::sleep(Duration::from_millis(500));
            }
        })
        .unwrap();

    jh0.join().expect("Joining UDP TMTC server thread failed");
    jh1.join().expect("Joining TM Funnel thread failed");
    jh2.join().expect("Joining Event Manager thread failed");
    jh3.join().expect("Joining AOCS thread failed");
}

pub fn update_time(time_provider: &mut TimeProvider, timestamp: &mut [u8]) {
    time_provider
        .update_from_now()
        .expect("Could not get current time");
    time_provider
        .write_to_bytes(timestamp)
        .expect("Writing timestamp failed");
}
