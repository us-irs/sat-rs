mod acs;
mod ccsds;
mod events;
mod hk;
mod logger;
mod pus;
mod requests;
mod tcp;
mod tm_funnel;
mod tmtc;
mod udp;

use crate::events::EventHandler;
use crate::pus::stack::PusStack;
use crate::tm_funnel::{TmFunnelDynamic, TmFunnelStatic};
use log::info;
use pus::test::create_test_service_dynamic;
use satrs_core::hal::std::tcp_server::ServerConfig;
use satrs_core::hal::std::udp_server::UdpTcServer;
use satrs_example::config::pool::{create_sched_tc_pool, create_static_pools};
use satrs_example::config::tasks::{
    FREQ_MS_AOCS, FREQ_MS_EVENT_HANDLING, FREQ_MS_PUS_STACK, FREQ_MS_UDP_TMTC,
};
use satrs_example::config::{RequestTargetId, TmSenderId, OBSW_SERVER_ADDR, PUS_APID, SERVER_PORT};
use tmtc::PusTcSourceProviderDynamic;
use udp::DynamicUdpTmHandler;

use crate::acs::AcsTask;
use crate::ccsds::CcsdsReceiver;
use crate::logger::setup_logger;
use crate::pus::action::{create_action_service_dynamic, create_action_service_static};
use crate::pus::event::{create_event_service_dynamic, create_event_service_static};
use crate::pus::hk::{create_hk_service_dynamic, create_hk_service_static};
use crate::pus::scheduler::{create_scheduler_service_dynamic, create_scheduler_service_static};
use crate::pus::test::create_test_service_static;
use crate::pus::{PusReceiver, PusTcMpscRouter};
use crate::requests::RequestWithToken;
use crate::tcp::{SyncTcpTmSource, TcpTask};
use crate::tmtc::{
    PusTcSourceProviderSharedPool, SharedTcPool, TcArgs, TmArgs, TmtcTaskDynamic, TmtcTaskStatic,
};
use crate::udp::{StaticUdpTmHandler, UdpTmtcServer};
use satrs_core::pus::event_man::EventRequestWithToken;
use satrs_core::pus::verification::{VerificationReporterCfg, VerificationReporterWithSender};
use satrs_core::pus::{EcssTmSender, MpscTmAsVecSender, MpscTmInStoreSender};
use satrs_core::spacepackets::{time::cds::TimeProvider, time::TimeWriter};
use satrs_core::tmtc::tm_helper::SharedTmStore;
use satrs_core::tmtc::{CcsdsDistributor, TargetId};
use satrs_core::ChannelId;
use satrs_example::TargetIdWithApid;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{self, channel};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

fn create_verification_reporter(verif_sender: impl EcssTmSender) -> VerificationReporterWithSender {
    let verif_cfg = VerificationReporterCfg::new(PUS_APID, 1, 2, 8).unwrap();
    // Every software component which needs to generate verification telemetry, gets a cloned
    // verification reporter.
    VerificationReporterWithSender::new(&verif_cfg, Box::new(verif_sender))
}

#[allow(dead_code)]
fn static_tmtc_pool_main() {
    let (tm_pool, tc_pool) = create_static_pools();
    let shared_tm_store = SharedTmStore::new(tm_pool);
    let shared_tc_pool = SharedTcPool {
        pool: Arc::new(RwLock::new(tc_pool)),
    };
    let (tc_source_tx, tc_source_rx) = channel();
    let (tm_funnel_tx, tm_funnel_rx) = channel();
    let (tm_server_tx, tm_server_rx) = channel();

    // Every software component which needs to generate verification telemetry, receives a cloned
    // verification reporter.
    let verif_reporter = create_verification_reporter(MpscTmInStoreSender::new(
        TmSenderId::PusVerification as ChannelId,
        "verif_sender",
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
    ));

    let acs_target_id = TargetIdWithApid::new(PUS_APID, RequestTargetId::AcsSubsystem as TargetId);
    let (acs_thread_tx, acs_thread_rx) = channel::<RequestWithToken>();
    // Some request are targetable. This map is used to retrieve sender handles based on a target ID.
    let mut request_map = HashMap::new();
    request_map.insert(acs_target_id, acs_thread_tx);

    let tc_source_wrapper = PusTcSourceProviderSharedPool {
        shared_pool: shared_tc_pool.clone(),
        tc_source: tc_source_tx,
    };

    // Create clones here to allow moving the values
    let tc_args = TcArgs {
        tc_source: tc_source_wrapper.clone(),
        tc_receiver: tc_source_rx,
    };
    let tm_args = TmArgs {
        tm_store: shared_tm_store.clone(),
        tm_sink_sender: tm_funnel_tx.clone(),
        tm_udp_server_rx: tm_server_rx,
    };

    // Create event handling components
    // These sender handles are used to send event requests, for example to enable or disable
    // certain events.
    let (event_request_tx, event_request_rx) = mpsc::channel::<EventRequestWithToken>();

    // The event task is the core handler to perform the event routing and TM handling as specified
    // in the sat-rs documentation.
    let mut event_handler = EventHandler::new(
        MpscTmInStoreSender::new(
            TmSenderId::AllEvents as ChannelId,
            "ALL_EVENTS_TX",
            shared_tm_store.clone(),
            tm_funnel_tx.clone(),
        ),
        verif_reporter.clone(),
        event_request_rx,
    );

    let (pus_test_tx, pus_test_rx) = channel();
    let (pus_event_tx, pus_event_rx) = channel();
    let (pus_sched_tx, pus_sched_rx) = channel();
    let (pus_hk_tx, pus_hk_rx) = channel();
    let (pus_action_tx, pus_action_rx) = channel();
    let pus_router = PusTcMpscRouter {
        test_service_receiver: pus_test_tx,
        event_service_receiver: pus_event_tx,
        sched_service_receiver: pus_sched_tx,
        hk_service_receiver: pus_hk_tx,
        action_service_receiver: pus_action_tx,
    };
    let pus_test_service = create_test_service_static(
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        shared_tc_pool.pool.clone(),
        event_handler.clone_event_sender(),
        pus_test_rx,
    );
    let pus_scheduler_service = create_scheduler_service_static(
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        tc_source_wrapper,
        pus_sched_rx,
        create_sched_tc_pool(),
    );
    let pus_event_service = create_event_service_static(
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        shared_tc_pool.pool.clone(),
        pus_event_rx,
        event_request_tx,
    );
    let pus_action_service = create_action_service_static(
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        shared_tc_pool.pool.clone(),
        pus_action_rx,
        request_map.clone(),
    );
    let pus_hk_service = create_hk_service_static(
        shared_tm_store.clone(),
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        shared_tc_pool.pool.clone(),
        pus_hk_rx,
        request_map,
    );
    let mut pus_stack = PusStack::new(
        pus_hk_service,
        pus_event_service,
        pus_action_service,
        pus_scheduler_service,
        pus_test_service,
    );

    let ccsds_receiver = CcsdsReceiver {
        tc_source: tc_args.tc_source.clone(),
    };
    let mut tmtc_task = TmtcTaskStatic::new(
        tc_args,
        PusReceiver::new(verif_reporter.clone(), pus_router),
    );

    let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let udp_ccsds_distributor = CcsdsDistributor::new(Box::new(ccsds_receiver.clone()));
    let udp_tc_server = UdpTcServer::new(sock_addr, 2048, Box::new(udp_ccsds_distributor))
        .expect("creating UDP TMTC server failed");
    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_handler: StaticUdpTmHandler {
            tm_rx: tm_args.tm_udp_server_rx,
            tm_store: tm_args.tm_store.clone_backing_pool(),
        },
    };

    let tcp_ccsds_distributor = CcsdsDistributor::new(Box::new(ccsds_receiver));
    let tcp_server_cfg = ServerConfig::new(sock_addr, Duration::from_millis(400), 4096, 8192);
    let sync_tm_tcp_source = SyncTcpTmSource::new(200);
    let mut tcp_server = TcpTask::new(
        tcp_server_cfg,
        sync_tm_tcp_source.clone(),
        tcp_ccsds_distributor,
    )
    .expect("tcp server creation failed");

    let mut acs_task = AcsTask::new(
        MpscTmInStoreSender::new(
            TmSenderId::AcsSubsystem as ChannelId,
            "ACS_TASK_SENDER",
            shared_tm_store.clone(),
            tm_funnel_tx.clone(),
        ),
        acs_thread_rx,
        verif_reporter,
    );

    let mut tm_funnel = TmFunnelStatic::new(
        shared_tm_store,
        sync_tm_tcp_source,
        tm_funnel_rx,
        tm_server_tx,
    );

    info!("Starting TMTC and UDP task");
    let jh_udp_tmtc = thread::Builder::new()
        .name("TMTC and UDP".to_string())
        .spawn(move || {
            info!("Running UDP server on port {SERVER_PORT}");
            loop {
                udp_tmtc_server.periodic_operation();
                tmtc_task.periodic_operation();
                thread::sleep(Duration::from_millis(FREQ_MS_UDP_TMTC));
            }
        })
        .unwrap();

    info!("Starting TCP task");
    let jh_tcp = thread::Builder::new()
        .name("TCP".to_string())
        .spawn(move || {
            info!("Running TCP server on port {SERVER_PORT}");
            loop {
                tcp_server.periodic_operation();
            }
        })
        .unwrap();

    info!("Starting TM funnel task");
    let jh_tm_funnel = thread::Builder::new()
        .name("TM Funnel".to_string())
        .spawn(move || loop {
            tm_funnel.operation();
        })
        .unwrap();

    info!("Starting event handling task");
    let jh_event_handling = thread::Builder::new()
        .name("Event".to_string())
        .spawn(move || loop {
            event_handler.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_EVENT_HANDLING));
        })
        .unwrap();

    info!("Starting AOCS thread");
    let jh_aocs = thread::Builder::new()
        .name("AOCS".to_string())
        .spawn(move || loop {
            acs_task.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_AOCS));
        })
        .unwrap();

    info!("Starting PUS handler thread");
    let jh_pus_handler = thread::Builder::new()
        .name("PUS".to_string())
        .spawn(move || loop {
            pus_stack.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_PUS_STACK));
        })
        .unwrap();

    jh_udp_tmtc
        .join()
        .expect("Joining UDP TMTC server thread failed");
    jh_tcp
        .join()
        .expect("Joining TCP TMTC server thread failed");
    jh_tm_funnel
        .join()
        .expect("Joining TM Funnel thread failed");
    jh_event_handling
        .join()
        .expect("Joining Event Manager thread failed");
    jh_aocs.join().expect("Joining AOCS thread failed");
    jh_pus_handler
        .join()
        .expect("Joining PUS handler thread failed");
}

#[allow(dead_code)]
fn dyn_tmtc_pool_main() {
    let (tc_source_tx, tc_source_rx) = channel();
    let (tm_funnel_tx, tm_funnel_rx) = channel();
    let (tm_server_tx, tm_server_rx) = channel();
    // Every software component which needs to generate verification telemetry, gets a cloned
    // verification reporter.
    let verif_reporter = create_verification_reporter(MpscTmAsVecSender::new(
        TmSenderId::PusVerification as ChannelId,
        "verif_sender",
        tm_funnel_tx.clone(),
    ));

    let acs_target_id = TargetIdWithApid::new(PUS_APID, RequestTargetId::AcsSubsystem as TargetId);
    let (acs_thread_tx, acs_thread_rx) = channel::<RequestWithToken>();
    // Some request are targetable. This map is used to retrieve sender handles based on a target ID.
    let mut request_map = HashMap::new();
    request_map.insert(acs_target_id, acs_thread_tx);

    let tc_source = PusTcSourceProviderDynamic(tc_source_tx);

    // Create event handling components
    // These sender handles are used to send event requests, for example to enable or disable
    // certain events.
    let (event_request_tx, event_request_rx) = mpsc::channel::<EventRequestWithToken>();
    // The event task is the core handler to perform the event routing and TM handling as specified
    // in the sat-rs documentation.
    let mut event_handler = EventHandler::new(
        MpscTmAsVecSender::new(
            TmSenderId::AllEvents as ChannelId,
            "ALL_EVENTS_TX",
            tm_funnel_tx.clone(),
        ),
        verif_reporter.clone(),
        event_request_rx,
    );

    let (pus_test_tx, pus_test_rx) = channel();
    let (pus_event_tx, pus_event_rx) = channel();
    let (pus_sched_tx, pus_sched_rx) = channel();
    let (pus_hk_tx, pus_hk_rx) = channel();
    let (pus_action_tx, pus_action_rx) = channel();
    let pus_router = PusTcMpscRouter {
        test_service_receiver: pus_test_tx,
        event_service_receiver: pus_event_tx,
        sched_service_receiver: pus_sched_tx,
        hk_service_receiver: pus_hk_tx,
        action_service_receiver: pus_action_tx,
    };

    let pus_test_service = create_test_service_dynamic(
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        event_handler.clone_event_sender(),
        pus_test_rx,
    );
    let pus_scheduler_service = create_scheduler_service_dynamic(
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        tc_source.0.clone(),
        pus_sched_rx,
        create_sched_tc_pool(),
    );

    let pus_event_service = create_event_service_dynamic(
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        pus_event_rx,
        event_request_tx,
    );
    let pus_action_service = create_action_service_dynamic(
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        pus_action_rx,
        request_map.clone(),
    );
    let pus_hk_service = create_hk_service_dynamic(
        tm_funnel_tx.clone(),
        verif_reporter.clone(),
        pus_hk_rx,
        request_map,
    );
    let mut pus_stack = PusStack::new(
        pus_hk_service,
        pus_event_service,
        pus_action_service,
        pus_scheduler_service,
        pus_test_service,
    );

    let ccsds_receiver = CcsdsReceiver { tc_source };

    let mut tmtc_task = TmtcTaskDynamic::new(
        tc_source_rx,
        PusReceiver::new(verif_reporter.clone(), pus_router),
    );

    let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let udp_ccsds_distributor = CcsdsDistributor::new(Box::new(ccsds_receiver.clone()));
    let udp_tc_server = UdpTcServer::new(sock_addr, 2048, Box::new(udp_ccsds_distributor))
        .expect("creating UDP TMTC server failed");
    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_handler: DynamicUdpTmHandler {
            tm_rx: tm_server_rx,
        },
    };

    let tcp_ccsds_distributor = CcsdsDistributor::new(Box::new(ccsds_receiver));
    let tcp_server_cfg = ServerConfig::new(sock_addr, Duration::from_millis(400), 4096, 8192);
    let sync_tm_tcp_source = SyncTcpTmSource::new(200);
    let mut tcp_server = TcpTask::new(
        tcp_server_cfg,
        sync_tm_tcp_source.clone(),
        tcp_ccsds_distributor,
    )
    .expect("tcp server creation failed");

    let mut acs_task = AcsTask::new(
        MpscTmAsVecSender::new(
            TmSenderId::AcsSubsystem as ChannelId,
            "ACS_TASK_SENDER",
            tm_funnel_tx.clone(),
        ),
        acs_thread_rx,
        verif_reporter,
    );
    let mut tm_funnel = TmFunnelDynamic::new(sync_tm_tcp_source, tm_funnel_rx, tm_server_tx);

    info!("Starting TMTC and UDP task");
    let jh_udp_tmtc = thread::Builder::new()
        .name("TMTC and UDP".to_string())
        .spawn(move || {
            info!("Running UDP server on port {SERVER_PORT}");
            loop {
                udp_tmtc_server.periodic_operation();
                tmtc_task.periodic_operation();
                thread::sleep(Duration::from_millis(FREQ_MS_UDP_TMTC));
            }
        })
        .unwrap();

    info!("Starting TCP task");
    let jh_tcp = thread::Builder::new()
        .name("TCP".to_string())
        .spawn(move || {
            info!("Running TCP server on port {SERVER_PORT}");
            loop {
                tcp_server.periodic_operation();
            }
        })
        .unwrap();

    info!("Starting TM funnel task");
    let jh_tm_funnel = thread::Builder::new()
        .name("TM Funnel".to_string())
        .spawn(move || loop {
            tm_funnel.operation();
        })
        .unwrap();

    info!("Starting event handling task");
    let jh_event_handling = thread::Builder::new()
        .name("Event".to_string())
        .spawn(move || loop {
            event_handler.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_EVENT_HANDLING));
        })
        .unwrap();

    info!("Starting AOCS thread");
    let jh_aocs = thread::Builder::new()
        .name("AOCS".to_string())
        .spawn(move || loop {
            acs_task.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_AOCS));
        })
        .unwrap();

    info!("Starting PUS handler thread");
    let jh_pus_handler = thread::Builder::new()
        .name("PUS".to_string())
        .spawn(move || loop {
            pus_stack.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_PUS_STACK));
        })
        .unwrap();

    jh_udp_tmtc
        .join()
        .expect("Joining UDP TMTC server thread failed");
    jh_tcp
        .join()
        .expect("Joining TCP TMTC server thread failed");
    jh_tm_funnel
        .join()
        .expect("Joining TM Funnel thread failed");
    jh_event_handling
        .join()
        .expect("Joining Event Manager thread failed");
    jh_aocs.join().expect("Joining AOCS thread failed");
    jh_pus_handler
        .join()
        .expect("Joining PUS handler thread failed");
}

fn main() {
    setup_logger().expect("setting up logging with fern failed");
    println!("Running OBSW example");
    #[cfg(not(feature = "dyn_tmtc"))]
    static_tmtc_pool_main();
    #[cfg(feature = "dyn_tmtc")]
    dyn_tmtc_pool_main();
}

pub fn update_time(time_provider: &mut TimeProvider, timestamp: &mut [u8]) {
    time_provider
        .update_from_now()
        .expect("Could not get current time");
    time_provider
        .write_to_bytes(timestamp)
        .expect("Writing timestamp failed");
}
