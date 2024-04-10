mod acs;
mod events;
mod hk;
mod interface;
mod logger;
mod pus;
mod requests;
mod tmtc;

use crate::events::EventHandler;
use crate::interface::udp::DynamicUdpTmHandler;
use crate::pus::stack::PusStack;
use crate::tmtc::tm_funnel::{TmFunnelDynamic, TmFunnelStatic};
use log::info;
use pus::test::create_test_service_dynamic;
use satrs::hal::std::tcp_server::ServerConfig;
use satrs::hal::std::udp_server::UdpTcServer;
use satrs::request::GenericMessage;
use satrs::tmtc::tm_helper::SharedTmPool;
use satrs_example::config::pool::{create_sched_tc_pool, create_static_pools};
use satrs_example::config::tasks::{
    FREQ_MS_AOCS, FREQ_MS_EVENT_HANDLING, FREQ_MS_PUS_STACK, FREQ_MS_UDP_TMTC,
};
use satrs_example::config::{OBSW_SERVER_ADDR, PACKET_ID_VALIDATOR, SERVER_PORT};
use tmtc::PusTcSourceProviderDynamic;

use crate::acs::mgm::{MgmHandlerLis3Mdl, MpscModeLeafInterface, SpiDummyInterface};
use crate::interface::tcp::{SyncTcpTmSource, TcpTask};
use crate::interface::udp::{StaticUdpTmHandler, UdpTmtcServer};
use crate::logger::setup_logger;
use crate::pus::action::{create_action_service_dynamic, create_action_service_static};
use crate::pus::event::{create_event_service_dynamic, create_event_service_static};
use crate::pus::hk::{create_hk_service_dynamic, create_hk_service_static};
use crate::pus::mode::{create_mode_service_dynamic, create_mode_service_static};
use crate::pus::scheduler::{create_scheduler_service_dynamic, create_scheduler_service_static};
use crate::pus::test::create_test_service_static;
use crate::pus::{PusReceiver, PusTcMpscRouter};
use crate::requests::{CompositeRequest, GenericRequestRouter};
use crate::tmtc::ccsds::CcsdsReceiver;
use crate::tmtc::{
    PusTcSourceProviderSharedPool, SharedTcPool, TcSourceTaskDynamic, TcSourceTaskStatic,
};
use satrs::mode::ModeRequest;
use satrs::pus::event_man::EventRequestWithToken;
use satrs::pus::TmInSharedPoolSender;
use satrs::spacepackets::{time::cds::CdsTime, time::TimeWriter};
use satrs::tmtc::CcsdsDistributor;
use satrs_example::config::components::MGM_HANDLER_0;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
fn static_tmtc_pool_main() {
    let (tm_pool, tc_pool) = create_static_pools();
    let shared_tm_pool = SharedTmPool::new(tm_pool);
    let shared_tc_pool = SharedTcPool {
        pool: Arc::new(RwLock::new(tc_pool)),
    };
    let (tc_source_tx, tc_source_rx) = mpsc::sync_channel(50);
    let (tm_funnel_tx, tm_funnel_rx) = mpsc::sync_channel(50);
    let (tm_server_tx, tm_server_rx) = mpsc::sync_channel(50);

    let tm_funnel_tx_sender =
        TmInSharedPoolSender::new(shared_tm_pool.clone(), tm_funnel_tx.clone());

    let (mgm_handler_composite_tx, mgm_handler_composite_rx) =
        mpsc::channel::<GenericMessage<CompositeRequest>>();
    let (mgm_handler_mode_tx, mgm_handler_mode_rx) = mpsc::channel::<GenericMessage<ModeRequest>>();

    // Some request are targetable. This map is used to retrieve sender handles based on a target ID.
    let mut request_map = GenericRequestRouter::default();
    request_map
        .composite_router_map
        .insert(MGM_HANDLER_0.id(), mgm_handler_composite_tx);
    request_map
        .mode_router_map
        .insert(MGM_HANDLER_0.id(), mgm_handler_mode_tx);

    // This helper structure is used by all telecommand providers which need to send telecommands
    // to the TC source.
    let tc_source = PusTcSourceProviderSharedPool {
        shared_pool: shared_tc_pool.clone(),
        tc_source: tc_source_tx,
    };

    // Create event handling components
    // These sender handles are used to send event requests, for example to enable or disable
    // certain events.
    let (event_request_tx, event_request_rx) = mpsc::channel::<EventRequestWithToken>();

    // The event task is the core handler to perform the event routing and TM handling as specified
    // in the sat-rs documentation.
    let mut event_handler = EventHandler::new(tm_funnel_tx.clone(), event_request_rx);

    let (pus_test_tx, pus_test_rx) = mpsc::channel();
    let (pus_event_tx, pus_event_rx) = mpsc::channel();
    let (pus_sched_tx, pus_sched_rx) = mpsc::channel();
    let (pus_hk_tx, pus_hk_rx) = mpsc::channel();
    let (pus_action_tx, pus_action_rx) = mpsc::channel();
    let (pus_mode_tx, pus_mode_rx) = mpsc::channel();

    let (_pus_action_reply_tx, pus_action_reply_rx) = mpsc::channel();
    let (pus_hk_reply_tx, pus_hk_reply_rx) = mpsc::channel();
    let (pus_mode_reply_tx, pus_mode_reply_rx) = mpsc::channel();

    let pus_router = PusTcMpscRouter {
        test_tc_sender: pus_test_tx,
        event_tc_sender: pus_event_tx,
        sched_tc_sender: pus_sched_tx,
        hk_tc_sender: pus_hk_tx,
        action_tc_sender: pus_action_tx,
        mode_tc_sender: pus_mode_tx,
    };
    let pus_test_service = create_test_service_static(
        tm_funnel_tx_sender.clone(),
        shared_tc_pool.pool.clone(),
        event_handler.clone_event_sender(),
        pus_test_rx,
    );
    let pus_scheduler_service = create_scheduler_service_static(
        tm_funnel_tx_sender.clone(),
        tc_source.clone(),
        pus_sched_rx,
        create_sched_tc_pool(),
    );
    let pus_event_service = create_event_service_static(
        tm_funnel_tx_sender.clone(),
        shared_tc_pool.pool.clone(),
        pus_event_rx,
        event_request_tx,
    );
    let pus_action_service = create_action_service_static(
        tm_funnel_tx_sender.clone(),
        shared_tc_pool.pool.clone(),
        pus_action_rx,
        request_map.clone(),
        pus_action_reply_rx,
    );
    let pus_hk_service = create_hk_service_static(
        tm_funnel_tx_sender.clone(),
        shared_tc_pool.pool.clone(),
        pus_hk_rx,
        request_map.clone(),
        pus_hk_reply_rx,
    );
    let pus_mode_service = create_mode_service_static(
        tm_funnel_tx_sender.clone(),
        shared_tc_pool.pool.clone(),
        pus_mode_rx,
        request_map,
        pus_mode_reply_rx,
    );
    let mut pus_stack = PusStack::new(
        pus_test_service,
        pus_hk_service,
        pus_event_service,
        pus_action_service,
        pus_scheduler_service,
        pus_mode_service,
    );

    let ccsds_receiver = CcsdsReceiver { tc_source };
    let mut tmtc_task = TcSourceTaskStatic::new(
        shared_tc_pool.clone(),
        tc_source_rx,
        PusReceiver::new(tm_funnel_tx_sender, pus_router),
    );

    let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let udp_ccsds_distributor = CcsdsDistributor::new(ccsds_receiver.clone());
    let udp_tc_server = UdpTcServer::new(sock_addr, 2048, Box::new(udp_ccsds_distributor))
        .expect("creating UDP TMTC server failed");
    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_handler: StaticUdpTmHandler {
            tm_rx: tm_server_rx,
            tm_store: shared_tm_pool.clone_backing_pool(),
        },
    };

    let tcp_ccsds_distributor = CcsdsDistributor::new(ccsds_receiver);
    let tcp_server_cfg = ServerConfig::new(sock_addr, Duration::from_millis(400), 4096, 8192);
    let sync_tm_tcp_source = SyncTcpTmSource::new(200);
    let mut tcp_server = TcpTask::new(
        tcp_server_cfg,
        sync_tm_tcp_source.clone(),
        tcp_ccsds_distributor,
        PACKET_ID_VALIDATOR.clone(),
    )
    .expect("tcp server creation failed");

    let mut tm_funnel = TmFunnelStatic::new(
        shared_tm_pool,
        sync_tm_tcp_source,
        tm_funnel_rx,
        tm_server_tx,
    );

    let (mgm_handler_mode_reply_to_parent_tx, _mgm_handler_mode_reply_to_parent_rx) =
        mpsc::channel();

    let dummy_spi_interface = SpiDummyInterface::default();
    let shared_mgm_set = Arc::default();
    let mode_leaf_interface = MpscModeLeafInterface {
        request_rx: mgm_handler_mode_rx,
        reply_tx_to_pus: pus_mode_reply_tx,
        reply_tx_to_parent: mgm_handler_mode_reply_to_parent_tx,
    };
    let mut mgm_handler = MgmHandlerLis3Mdl::new(
        MGM_HANDLER_0,
        "MGM_0",
        mode_leaf_interface,
        mgm_handler_composite_rx,
        pus_hk_reply_tx,
        tm_funnel_tx,
        dummy_spi_interface,
        shared_mgm_set,
    );

    info!("Starting TMTC and UDP task");
    let jh_udp_tmtc = thread::Builder::new()
        .name("SATRS tmtc-udp".to_string())
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
        .name("sat-rs tcp".to_string())
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
        .name("sat-rs events".to_string())
        .spawn(move || loop {
            event_handler.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_EVENT_HANDLING));
        })
        .unwrap();

    info!("Starting AOCS thread");
    let jh_aocs = thread::Builder::new()
        .name("sat-rs aocs".to_string())
        .spawn(move || loop {
            mgm_handler.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_AOCS));
        })
        .unwrap();

    info!("Starting PUS handler thread");
    let jh_pus_handler = thread::Builder::new()
        .name("sat-rs pus".to_string())
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
    let (tc_source_tx, tc_source_rx) = mpsc::channel();
    let (tm_funnel_tx, tm_funnel_rx) = mpsc::channel();
    let (tm_server_tx, tm_server_rx) = mpsc::channel();

    // Some request are targetable. This map is used to retrieve sender handles based on a target ID.
    let (mgm_handler_composite_tx, mgm_handler_composite_rx) =
        mpsc::channel::<GenericMessage<CompositeRequest>>();
    let (mgm_handler_mode_tx, mgm_handler_mode_rx) = mpsc::channel::<GenericMessage<ModeRequest>>();

    // Some request are targetable. This map is used to retrieve sender handles based on a target ID.
    let mut request_map = GenericRequestRouter::default();
    request_map
        .composite_router_map
        .insert(MGM_HANDLER_0.raw(), mgm_handler_composite_tx);
    request_map
        .mode_router_map
        .insert(MGM_HANDLER_0.raw(), mgm_handler_mode_tx);

    let tc_source = PusTcSourceProviderDynamic(tc_source_tx);

    // Create event handling components
    // These sender handles are used to send event requests, for example to enable or disable
    // certain events.
    let (event_request_tx, event_request_rx) = mpsc::channel::<EventRequestWithToken>();
    // The event task is the core handler to perform the event routing and TM handling as specified
    // in the sat-rs documentation.
    let mut event_handler = EventHandler::new(tm_funnel_tx.clone(), event_request_rx);

    let (pus_test_tx, pus_test_rx) = mpsc::channel();
    let (pus_event_tx, pus_event_rx) = mpsc::channel();
    let (pus_sched_tx, pus_sched_rx) = mpsc::channel();
    let (pus_hk_tx, pus_hk_rx) = mpsc::channel();
    let (pus_action_tx, pus_action_rx) = mpsc::channel();
    let (pus_mode_tx, pus_mode_rx) = mpsc::channel();

    let (_pus_action_reply_tx, pus_action_reply_rx) = mpsc::channel();
    let (pus_hk_reply_tx, pus_hk_reply_rx) = mpsc::channel();
    let (pus_mode_reply_tx, pus_mode_reply_rx) = mpsc::channel();

    let pus_router = PusTcMpscRouter {
        test_tc_sender: pus_test_tx,
        event_tc_sender: pus_event_tx,
        sched_tc_sender: pus_sched_tx,
        hk_tc_sender: pus_hk_tx,
        action_tc_sender: pus_action_tx,
        mode_tc_sender: pus_mode_tx,
    };

    let pus_test_service = create_test_service_dynamic(
        tm_funnel_tx.clone(),
        event_handler.clone_event_sender(),
        pus_test_rx,
    );
    let pus_scheduler_service = create_scheduler_service_dynamic(
        tm_funnel_tx.clone(),
        tc_source.0.clone(),
        pus_sched_rx,
        create_sched_tc_pool(),
    );

    let pus_event_service =
        create_event_service_dynamic(tm_funnel_tx.clone(), pus_event_rx, event_request_tx);
    let pus_action_service = create_action_service_dynamic(
        tm_funnel_tx.clone(),
        pus_action_rx,
        request_map.clone(),
        pus_action_reply_rx,
    );
    let pus_hk_service = create_hk_service_dynamic(
        tm_funnel_tx.clone(),
        pus_hk_rx,
        request_map.clone(),
        pus_hk_reply_rx,
    );
    let pus_mode_service = create_mode_service_dynamic(
        tm_funnel_tx.clone(),
        pus_mode_rx,
        request_map,
        pus_mode_reply_rx,
    );
    let mut pus_stack = PusStack::new(
        pus_test_service,
        pus_hk_service,
        pus_event_service,
        pus_action_service,
        pus_scheduler_service,
        pus_mode_service,
    );

    let ccsds_receiver = CcsdsReceiver { tc_source };

    let mut tmtc_task = TcSourceTaskDynamic::new(
        tc_source_rx,
        PusReceiver::new(tm_funnel_tx.clone(), pus_router),
    );

    let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let udp_ccsds_distributor = CcsdsDistributor::new(ccsds_receiver.clone());
    let udp_tc_server = UdpTcServer::new(sock_addr, 2048, Box::new(udp_ccsds_distributor))
        .expect("creating UDP TMTC server failed");
    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_handler: DynamicUdpTmHandler {
            tm_rx: tm_server_rx,
        },
    };

    let tcp_ccsds_distributor = CcsdsDistributor::new(ccsds_receiver);
    let tcp_server_cfg = ServerConfig::new(sock_addr, Duration::from_millis(400), 4096, 8192);
    let sync_tm_tcp_source = SyncTcpTmSource::new(200);
    let mut tcp_server = TcpTask::new(
        tcp_server_cfg,
        sync_tm_tcp_source.clone(),
        tcp_ccsds_distributor,
        PACKET_ID_VALIDATOR.clone(),
    )
    .expect("tcp server creation failed");

    let mut tm_funnel = TmFunnelDynamic::new(sync_tm_tcp_source, tm_funnel_rx, tm_server_tx);

    let (mgm_handler_mode_reply_to_parent_tx, _mgm_handler_mode_reply_to_parent_rx) =
        mpsc::channel();
    let dummy_spi_interface = SpiDummyInterface::default();
    let shared_mgm_set = Arc::default();
    let mode_leaf_interface = MpscModeLeafInterface {
        request_rx: mgm_handler_mode_rx,
        reply_tx_to_pus: pus_mode_reply_tx,
        reply_tx_to_parent: mgm_handler_mode_reply_to_parent_tx,
    };
    let mut mgm_handler = MgmHandlerLis3Mdl::new(
        MGM_HANDLER_0,
        "MGM_0",
        mode_leaf_interface,
        mgm_handler_composite_rx,
        pus_hk_reply_tx,
        tm_funnel_tx,
        dummy_spi_interface,
        shared_mgm_set,
    );

    info!("Starting TMTC and UDP task");
    let jh_udp_tmtc = thread::Builder::new()
        .name("sat-rs tmtc-udp".to_string())
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
        .name("sat-rs tcp".to_string())
        .spawn(move || {
            info!("Running TCP server on port {SERVER_PORT}");
            loop {
                tcp_server.periodic_operation();
            }
        })
        .unwrap();

    info!("Starting TM funnel task");
    let jh_tm_funnel = thread::Builder::new()
        .name("sat-rs tm-funnel".to_string())
        .spawn(move || loop {
            tm_funnel.operation();
        })
        .unwrap();

    info!("Starting event handling task");
    let jh_event_handling = thread::Builder::new()
        .name("sat-rs events".to_string())
        .spawn(move || loop {
            event_handler.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_EVENT_HANDLING));
        })
        .unwrap();

    info!("Starting AOCS thread");
    let jh_aocs = thread::Builder::new()
        .name("sat-rs aocs".to_string())
        .spawn(move || loop {
            mgm_handler.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_AOCS));
        })
        .unwrap();

    info!("Starting PUS handler thread");
    let jh_pus_handler = thread::Builder::new()
        .name("sat-rs pus".to_string())
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

pub fn update_time(time_provider: &mut CdsTime, timestamp: &mut [u8]) {
    time_provider
        .update_from_now()
        .expect("Could not get current time");
    time_provider
        .write_to_bytes(timestamp)
        .expect("Writing timestamp failed");
}
