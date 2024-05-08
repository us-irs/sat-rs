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
use crate::tmtc::tc_source::{TcSourceTaskDynamic, TcSourceTaskStatic};
use crate::tmtc::tm_sink::{TmSinkDynamic, TmSinkStatic};
use log::info;
use pus::test::create_test_service_dynamic;
use satrs::hal::std::tcp_server::ServerConfig;
use satrs::hal::std::udp_server::UdpTcServer;
use satrs::pus::HandlingStatus;
use satrs::request::GenericMessage;
use satrs::tmtc::{PacketSenderWithSharedPool, SharedPacketPool};
use satrs_example::config::pool::{create_sched_tc_pool, create_static_pools};
use satrs_example::config::tasks::{
    FREQ_MS_AOCS, FREQ_MS_PUS_STACK, FREQ_MS_UDP_TMTC, SIM_CLIENT_IDLE_DELAY_MS,
};
use satrs_example::config::{OBSW_SERVER_ADDR, PACKET_ID_VALIDATOR, SERVER_PORT};

use crate::acs::mgm::{
    MgmHandlerLis3Mdl, MpscModeLeafInterface, SpiDummyInterface, SpiSimInterface,
    SpiSimInterfaceWrapper,
};
use crate::interface::sim_client_udp::create_sim_client;
use crate::interface::tcp::{SyncTcpTmSource, TcpTask};
use crate::interface::udp::{StaticUdpTmHandler, UdpTmtcServer};
use crate::logger::setup_logger;
use crate::pus::action::{create_action_service_dynamic, create_action_service_static};
use crate::pus::event::{create_event_service_dynamic, create_event_service_static};
use crate::pus::hk::{create_hk_service_dynamic, create_hk_service_static};
use crate::pus::mode::{create_mode_service_dynamic, create_mode_service_static};
use crate::pus::scheduler::{create_scheduler_service_dynamic, create_scheduler_service_static};
use crate::pus::test::create_test_service_static;
use crate::pus::{PusTcDistributor, PusTcMpscRouter};
use crate::requests::{CompositeRequest, GenericRequestRouter};
use satrs::mode::ModeRequest;
use satrs::pus::event_man::EventRequestWithToken;
use satrs::spacepackets::{time::cds::CdsTime, time::TimeWriter};
use satrs_example::config::components::{MGM_HANDLER_0, TCP_SERVER, UDP_SERVER};
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
fn static_tmtc_pool_main() {
    let (tm_pool, tc_pool) = create_static_pools();
    let shared_tm_pool = Arc::new(RwLock::new(tm_pool));
    let shared_tc_pool = Arc::new(RwLock::new(tc_pool));
    let shared_tm_pool_wrapper = SharedPacketPool::new(&shared_tm_pool);
    let shared_tc_pool_wrapper = SharedPacketPool::new(&shared_tc_pool);
    let (tc_source_tx, tc_source_rx) = mpsc::sync_channel(50);
    let (tm_sink_tx, tm_sink_rx) = mpsc::sync_channel(50);
    let (tm_server_tx, tm_server_rx) = mpsc::sync_channel(50);

    let tm_sink_tx_sender =
        PacketSenderWithSharedPool::new(tm_sink_tx.clone(), shared_tm_pool_wrapper.clone());

    let (sim_request_tx, sim_request_rx) = mpsc::channel();
    let (mgm_sim_reply_tx, mgm_sim_reply_rx) = mpsc::channel();
    let mut opt_sim_client = create_sim_client(sim_request_rx);

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
    let tc_source = PacketSenderWithSharedPool::new(tc_source_tx, shared_tc_pool_wrapper.clone());

    // Create event handling components
    // These sender handles are used to send event requests, for example to enable or disable
    // certain events.
    let (event_tx, event_rx) = mpsc::sync_channel(100);
    let (event_request_tx, event_request_rx) = mpsc::channel::<EventRequestWithToken>();

    // The event task is the core handler to perform the event routing and TM handling as specified
    // in the sat-rs documentation.
    let mut event_handler = EventHandler::new(tm_sink_tx.clone(), event_rx, event_request_rx);

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
        tm_sink_tx_sender.clone(),
        shared_tc_pool.clone(),
        event_tx.clone(),
        pus_test_rx,
    );
    let pus_scheduler_service = create_scheduler_service_static(
        tm_sink_tx_sender.clone(),
        tc_source.clone(),
        pus_sched_rx,
        create_sched_tc_pool(),
    );
    let pus_event_service = create_event_service_static(
        tm_sink_tx_sender.clone(),
        shared_tc_pool.clone(),
        pus_event_rx,
        event_request_tx,
    );
    let pus_action_service = create_action_service_static(
        tm_sink_tx_sender.clone(),
        shared_tc_pool.clone(),
        pus_action_rx,
        request_map.clone(),
        pus_action_reply_rx,
    );
    let pus_hk_service = create_hk_service_static(
        tm_sink_tx_sender.clone(),
        shared_tc_pool.clone(),
        pus_hk_rx,
        request_map.clone(),
        pus_hk_reply_rx,
    );
    let pus_mode_service = create_mode_service_static(
        tm_sink_tx_sender.clone(),
        shared_tc_pool.clone(),
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

    let mut tmtc_task = TcSourceTaskStatic::new(
        shared_tc_pool_wrapper.clone(),
        tc_source_rx,
        PusTcDistributor::new(tm_sink_tx_sender, pus_router),
    );

    let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let udp_tc_server = UdpTcServer::new(UDP_SERVER.id(), sock_addr, 2048, tc_source.clone())
        .expect("creating UDP TMTC server failed");
    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_handler: StaticUdpTmHandler {
            tm_rx: tm_server_rx,
            tm_store: shared_tm_pool.clone(),
        },
    };

    let tcp_server_cfg = ServerConfig::new(
        TCP_SERVER.id(),
        sock_addr,
        Duration::from_millis(400),
        4096,
        8192,
    );
    let sync_tm_tcp_source = SyncTcpTmSource::new(200);
    let mut tcp_server = TcpTask::new(
        tcp_server_cfg,
        sync_tm_tcp_source.clone(),
        tc_source.clone(),
        PACKET_ID_VALIDATOR.clone(),
    )
    .expect("tcp server creation failed");

    let mut tm_sink = TmSinkStatic::new(
        shared_tm_pool_wrapper,
        sync_tm_tcp_source,
        tm_sink_rx,
        tm_server_tx,
    );

    let (mgm_handler_mode_reply_to_parent_tx, _mgm_handler_mode_reply_to_parent_rx) =
        mpsc::channel();

    let shared_mgm_set = Arc::default();
    let mode_leaf_interface = MpscModeLeafInterface {
        request_rx: mgm_handler_mode_rx,
        reply_tx_to_pus: pus_mode_reply_tx,
        reply_tx_to_parent: mgm_handler_mode_reply_to_parent_tx,
    };

    let mgm_spi_interface = if let Some(sim_client) = opt_sim_client.as_mut() {
        sim_client.add_reply_recipient(satrs_minisim::SimComponent::MgmLis3Mdl, mgm_sim_reply_tx);
        SpiSimInterfaceWrapper::Sim(SpiSimInterface {
            sim_request_tx: sim_request_tx.clone(),
            sim_reply_rx: mgm_sim_reply_rx,
        })
    } else {
        SpiSimInterfaceWrapper::Dummy(SpiDummyInterface::default())
    };
    let mut mgm_handler = MgmHandlerLis3Mdl::new(
        MGM_HANDLER_0,
        "MGM_0",
        mode_leaf_interface,
        mgm_handler_composite_rx,
        pus_hk_reply_tx,
        tm_sink_tx,
        mgm_spi_interface,
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
        .name("tm sink".to_string())
        .spawn(move || loop {
            tm_sink.operation();
        })
        .unwrap();

    let mut opt_jh_sim_client = None;
    if let Some(mut sim_client) = opt_sim_client {
        info!("Starting UDP sim client task");
        opt_jh_sim_client = Some(
            thread::Builder::new()
                .name("sat-rs sim adapter".to_string())
                .spawn(move || loop {
                    if sim_client.operation() == HandlingStatus::Empty {
                        std::thread::sleep(Duration::from_millis(SIM_CLIENT_IDLE_DELAY_MS));
                    }
                })
                .unwrap(),
        );
    }

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
            event_handler.periodic_operation();
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
    if let Some(jh_sim_client) = opt_jh_sim_client {
        jh_sim_client
            .join()
            .expect("Joining SIM client thread failed");
    }
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

    let (sim_request_tx, sim_request_rx) = mpsc::channel();
    let (mgm_sim_reply_tx, mgm_sim_reply_rx) = mpsc::channel();
    let mut opt_sim_client = create_sim_client(sim_request_rx);

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

    // Create event handling components
    // These sender handles are used to send event requests, for example to enable or disable
    // certain events.
    let (event_tx, event_rx) = mpsc::sync_channel(100);
    let (event_request_tx, event_request_rx) = mpsc::channel::<EventRequestWithToken>();
    // The event task is the core handler to perform the event routing and TM handling as specified
    // in the sat-rs documentation.
    let mut event_handler = EventHandler::new(tm_funnel_tx.clone(), event_rx, event_request_rx);

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

    let pus_test_service =
        create_test_service_dynamic(tm_funnel_tx.clone(), event_tx.clone(), pus_test_rx);
    let pus_scheduler_service = create_scheduler_service_dynamic(
        tm_funnel_tx.clone(),
        tc_source_tx.clone(),
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

    let mut tmtc_task = TcSourceTaskDynamic::new(
        tc_source_rx,
        PusTcDistributor::new(tm_funnel_tx.clone(), pus_router),
    );

    let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let udp_tc_server = UdpTcServer::new(UDP_SERVER.id(), sock_addr, 2048, tc_source_tx.clone())
        .expect("creating UDP TMTC server failed");
    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_handler: DynamicUdpTmHandler {
            tm_rx: tm_server_rx,
        },
    };

    let tcp_server_cfg = ServerConfig::new(
        TCP_SERVER.id(),
        sock_addr,
        Duration::from_millis(400),
        4096,
        8192,
    );
    let sync_tm_tcp_source = SyncTcpTmSource::new(200);
    let mut tcp_server = TcpTask::new(
        tcp_server_cfg,
        sync_tm_tcp_source.clone(),
        tc_source_tx.clone(),
        PACKET_ID_VALIDATOR.clone(),
    )
    .expect("tcp server creation failed");

    let mut tm_funnel = TmSinkDynamic::new(sync_tm_tcp_source, tm_funnel_rx, tm_server_tx);

    let (mgm_handler_mode_reply_to_parent_tx, _mgm_handler_mode_reply_to_parent_rx) =
        mpsc::channel();
    let shared_mgm_set = Arc::default();
    let mode_leaf_interface = MpscModeLeafInterface {
        request_rx: mgm_handler_mode_rx,
        reply_tx_to_pus: pus_mode_reply_tx,
        reply_tx_to_parent: mgm_handler_mode_reply_to_parent_tx,
    };

    let mgm_spi_interface = if let Some(sim_client) = opt_sim_client.as_mut() {
        sim_client.add_reply_recipient(satrs_minisim::SimComponent::MgmLis3Mdl, mgm_sim_reply_tx);
        SpiSimInterfaceWrapper::Sim(SpiSimInterface {
            sim_request_tx: sim_request_tx.clone(),
            sim_reply_rx: mgm_sim_reply_rx,
        })
    } else {
        SpiSimInterfaceWrapper::Dummy(SpiDummyInterface::default())
    };
    let mut mgm_handler = MgmHandlerLis3Mdl::new(
        MGM_HANDLER_0,
        "MGM_0",
        mode_leaf_interface,
        mgm_handler_composite_rx,
        pus_hk_reply_tx,
        tm_funnel_tx,
        mgm_spi_interface,
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
        .name("sat-rs tm-sink".to_string())
        .spawn(move || loop {
            tm_funnel.operation();
        })
        .unwrap();

    let mut opt_jh_sim_client = None;
    if let Some(mut sim_client) = opt_sim_client {
        info!("Starting UDP sim client task");
        opt_jh_sim_client = Some(
            thread::Builder::new()
                .name("sat-rs sim adapter".to_string())
                .spawn(move || loop {
                    if sim_client.operation() == HandlingStatus::Empty {
                        std::thread::sleep(Duration::from_millis(SIM_CLIENT_IDLE_DELAY_MS));
                    }
                })
                .unwrap(),
        );
    }

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
            event_handler.periodic_operation();
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
    if let Some(jh_sim_client) = opt_jh_sim_client {
        jh_sim_client
            .join()
            .expect("Joining SIM client thread failed");
    }
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
