mod ccsds;
mod hk;
mod pus;
mod requests;
mod tmtc;

use crate::hk::{AcsHkIds, HkRequest};
use crate::requests::{Request, RequestWithToken};
use crate::tmtc::{
    core_tmtc_task, OtherArgs, PusTcSource, TcArgs, TcStore, TmArgs, TmFunnel, TmStore, PUS_APID,
};
use satrs_core::event_man::{
    EventManagerWithMpscQueue, MpscEventReceiver, MpscEventU32SendProvider, SendEventProvider,
};
use satrs_core::events::EventU32;
use satrs_core::pool::{LocalPool, PoolCfg, StoreAddr};
use satrs_core::pus::event_man::{
    DefaultPusMgmtBackendProvider, EventReporter, EventRequest, EventRequestWithToken,
    PusEventDispatcher,
};
use satrs_core::pus::hk::Subservice;
use satrs_core::pus::verification::{
    MpscVerifSender, VerificationReporterCfg, VerificationReporterWithSender,
};
use satrs_core::pus::{EcssTmError, EcssTmSenderCore};
use satrs_core::seq_count::SimpleSeqCountProvider;
use satrs_example::{RequestTargetId, OBSW_SERVER_ADDR, SERVER_PORT};
use spacepackets::time::cds::TimeProvider;
use spacepackets::time::TimeWriter;
use spacepackets::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::{SequenceFlags, SpHeader};
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

    fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmError<Self::Error>> {
        let addr = self.store_helper.add_pus_tm(&tm);
        self.sender.send(addr).map_err(EcssTmError::SendError)
    }
}

fn main() {
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
        Box::new(SimpleSeqCountProvider::default()),
        1,
        2,
        8,
    )
    .unwrap();
    let verif_reporter = VerificationReporterWithSender::new(&verif_cfg, Box::new(verif_sender));

    // Create event handling components
    let (event_request_tx, event_request_rx) = channel::<EventRequestWithToken>();
    let (event_sender, event_man_rx) = channel();
    let event_recv = MpscEventReceiver::<EventU32>::new(event_man_rx);
    let mut event_man = EventManagerWithMpscQueue::new(Box::new(event_recv));
    let event_reporter = EventReporter::new(PUS_APID, 128).unwrap();
    let pus_tm_backend = DefaultPusMgmtBackendProvider::<EventU32>::default();
    let mut pus_event_dispatcher =
        PusEventDispatcher::new(event_reporter, Box::new(pus_tm_backend));
    let (pus_event_man_tx, pus_event_man_rx) = channel();
    let pus_event_man_send_provider = MpscEventU32SendProvider::new(1, pus_event_man_tx);
    let mut reporter_event_handler = verif_reporter.clone();
    let mut reporter_aocs = verif_reporter.clone();
    event_man.subscribe_all(pus_event_man_send_provider.id());

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

    println!("Starting TMTC task");
    let jh0 = thread::spawn(move || {
        core_tmtc_task(core_args, tc_args, tm_args);
    });

    println!("Starting TM funnel task");
    let jh1 = thread::spawn(move || {
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
    });

    println!("Starting event handling task");
    let jh2 = thread::spawn(move || {
        let mut timestamp: [u8; 7] = [0; 7];
        let mut sender = EventTmSender::new(tm_store, tm_funnel_tx);
        let mut time_provider = TimeProvider::new_with_u16_days(0, 0);
        let mut report_completion = |event_req: EventRequestWithToken, timestamp: &[u8]| {
            reporter_event_handler
                .completion_success(event_req.token, timestamp)
                .expect("Sending completion success failed");
        };
        loop {
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
            if let Ok((event, _param)) = pus_event_man_rx.try_recv() {
                update_time(&mut time_provider, &mut timestamp);
                pus_event_dispatcher
                    .generate_pus_event_tm_generic(&mut sender, &timestamp, event, None)
                    .expect("Sending TM as event failed");
            }
            thread::sleep(Duration::from_millis(400));
        }
    });

    println!("Starting AOCS thread");
    let jh3 = thread::spawn(move || {
        let mut timestamp: [u8; 7] = [0; 7];
        let mut time_provider = TimeProvider::new_with_u16_days(0, 0);
        loop {
            match acs_thread_rx.try_recv() {
                Ok(request) => {
                    println!("ACS thread: Received HK request {:?}", request.0);
                    update_time(&mut time_provider, &mut timestamp);
                    match request.0 {
                        Request::HkRequest(hk_req) => match hk_req {
                            HkRequest::OneShot(address) => {
                                assert_eq!(address.target_id, RequestTargetId::AcsSubsystem as u32);
                                if address.unique_id == AcsHkIds::TestMgmSet as u32 {
                                    let mut sp_header =
                                        SpHeader::tm(PUS_APID, SequenceFlags::Unsegmented, 0, 0)
                                            .unwrap();
                                    let sec_header = PusTmSecondaryHeader::new_simple(
                                        3,
                                        Subservice::TmHkPacket as u8,
                                        &timestamp,
                                    );
                                    let mut buf: [u8; 8] = [0; 8];
                                    address.write_to_be_bytes(&mut buf).unwrap();
                                    let pus_tm =
                                        PusTm::new(&mut sp_header, sec_header, Some(&buf), true);
                                    let addr = aocs_tm_store.add_pus_tm(&pus_tm);
                                    aocs_to_funnel.send(addr).expect("Sending HK TM failed");
                                }
                            }
                            HkRequest::Enable(_) => {}
                            HkRequest::Disable(_) => {}
                            HkRequest::ModifyCollectionInterval(_, _) => {}
                        },
                    }
                    let started_token = reporter_aocs
                        .start_success(request.1, &timestamp)
                        .expect("Sending start success failed");
                    reporter_aocs
                        .completion_success(started_token, &timestamp)
                        .expect("Sending completion success failed");
                }
                Err(e) => match e {
                    TryRecvError::Empty => {}
                    TryRecvError::Disconnected => {
                        println!("ACS thread: Message Queue TX disconnected!")
                    }
                },
            }
            thread::sleep(Duration::from_millis(500));
        }
    });

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
