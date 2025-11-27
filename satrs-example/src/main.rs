use std::{
    net::{IpAddr, SocketAddr},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use eps::{
    pcdu::{PcduHandler, SerialInterfaceDummy, SerialInterfaceToSim, SerialSimInterfaceWrapper},
    PowerSwitchHelper,
};
use interface::{
    sim_client_udp::create_sim_client,
    tcp::{SyncTcpTmSource, TcpTask},
    udp::UdpTmtcServer,
};
use log::info;
use logger::setup_logger;
use requests::GenericRequestRouter;
use satrs::{
    hal::std::{tcp_server::ServerConfig, udp_server::UdpTcServer},
    mode::{Mode, ModeAndSubmode, ModeRequest, ModeRequestHandlerMpscBounded},
    pus::{event_man::EventRequestWithToken, HandlingStatus},
    request::{GenericMessage, MessageMetadata},
    spacepackets::time::cds::CdsTime,
};
use satrs_example::{
    config::{
        components::NO_SENDER,
        tasks::{FREQ_MS_AOCS, FREQ_MS_PUS_STACK, FREQ_MS_UDP_TMTC, SIM_CLIENT_IDLE_DELAY_MS},
        OBSW_SERVER_ADDR, PACKET_ID_VALIDATOR, SERVER_PORT,
    },
    ids::{
        acs::*,
        eps::*,
        tmtc::{TCP_SERVER, UDP_SERVER},
    },
    DeviceMode,
};
use tmtc::sender::TmTcSender;
use tmtc::{tc_source::TcSourceTask, tm_sink::TmSink};

use crate::{interface::udp::UdpTmHandlerWithChannel, tmtc::tc_source::CcsdsDistributor};

mod acs;
mod eps;
mod events;
mod hk;
mod interface;
mod logger;
//mod pus;
mod requests;
mod spi;
mod tmtc;

fn main() {
    setup_logger().expect("setting up logging with fern failed");
    println!("Running OBSW example");

    let (tc_source_tx, tc_source_rx) = mpsc::sync_channel(50);
    let (tm_sink_tx, tm_sink_rx) = mpsc::sync_channel(50);
    let (tm_server_tx, tm_server_rx) = mpsc::sync_channel(50);

    let (sim_request_tx, sim_request_rx) = mpsc::channel();
    //let (mgm_0_sim_reply_tx, mgm_0_sim_reply_rx) = mpsc::channel();
    //let (mgm_1_sim_reply_tx, mgm_1_sim_reply_rx) = mpsc::channel();
    let (pcdu_sim_reply_tx, pcdu_sim_reply_rx) = mpsc::channel();
    let mut opt_sim_client = create_sim_client(sim_request_rx);

    let (mgm_0_handler_composite_tx, mgm_0_handler_composite_rx) = mpsc::sync_channel(10);
    let (mgm_1_handler_composite_tx, mgm_1_handler_composite_rx) = mpsc::sync_channel(10);
    let (pcdu_handler_composite_tx, pcdu_handler_composite_rx) = mpsc::sync_channel(30);
    let (mgm_0_handler_mode_tx, mgm_0_handler_mode_rx) = mpsc::sync_channel(5);
    let (mgm_1_handler_mode_tx, mgm_1_handler_mode_rx) = mpsc::sync_channel(5);
    let (pcdu_handler_mode_tx, pcdu_handler_mode_rx) = mpsc::sync_channel(5);

    // Some request are targetable. This map is used to retrieve sender handles based on a target ID.
    let mut request_map = GenericRequestRouter::default();
    request_map
        .composite_router_map
        .insert(MGM0.id(), mgm_0_handler_composite_tx);
    request_map
        .composite_router_map
        .insert(MGM1.id(), mgm_1_handler_composite_tx);
    request_map
        .composite_router_map
        .insert(PCDU.id(), pcdu_handler_composite_tx);

    // Create event handling components
    // These sender handles are used to send event requests, for example to enable or disable
    // certain events.
    //let (event_tx, event_rx) = mpsc::sync_channel(100);
    let (event_request_tx, event_request_rx) = mpsc::channel::<EventRequestWithToken>();

    // The event task is the core handler to perform the event routing and TM handling as specified
    // in the sat-rs documentation.
    //let mut event_handler = EventHandler::new(tm_sink_tx.clone(), event_rx, event_request_rx);

    //let (pus_test_tx, pus_test_rx) = mpsc::sync_channel(20);
    //let (pus_event_tx, pus_event_rx) = mpsc::sync_channel(10);
    //let (pus_sched_tx, pus_sched_rx) = mpsc::sync_channel(50);
    //let (pus_hk_tx, pus_hk_rx) = mpsc::sync_channel(50);
    //let (pus_action_tx, pus_action_rx) = mpsc::sync_channel(50);
    //let (pus_mode_tx, pus_mode_rx) = mpsc::sync_channel(50);

    //let (_pus_action_reply_tx, pus_action_reply_rx) = mpsc::channel();
    let (pus_hk_reply_tx, pus_hk_reply_rx) = mpsc::sync_channel(50);
    //let (pus_mode_reply_tx, pus_mode_reply_rx) = mpsc::sync_channel(30);

    let mut ccsds_distributor = CcsdsDistributor::default();
    let mut tmtc_task = TcSourceTask::new(
        tc_source_rx,
        ccsds_distributor,
        //PusTcDistributor::new(tm_sender.clone(), pus_router),
    );
    let tc_sender = TmTcSender::Normal(tc_source_tx.clone());
    let udp_tm_handler = UdpTmHandlerWithChannel {
        tm_rx: tm_server_rx,
    };

    let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let udp_tc_server = UdpTcServer::new(UDP_SERVER.id(), sock_addr, 2048, tc_sender.clone())
        .expect("creating UDP TMTC server failed");
    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_handler: udp_tm_handler.into(),
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
        tc_sender,
        PACKET_ID_VALIDATOR.clone(),
    )
    .expect("tcp server creation failed");

    let mut tm_sink = TmSink::new(sync_tm_tcp_source, tm_sink_rx, tm_server_tx);

    let shared_switch_set = Arc::new(Mutex::default());
    let (switch_request_tx, switch_request_rx) = mpsc::sync_channel(20);
    let switch_helper = PowerSwitchHelper::new(switch_request_tx, shared_switch_set.clone());

    //let shared_mgm_0_set = Arc::default();
    //let shared_mgm_1_set = Arc::default();
    let mgm_0_mode_node = ModeRequestHandlerMpscBounded::new(MGM0.into(), mgm_0_handler_mode_rx);
    let mgm_1_mode_node = ModeRequestHandlerMpscBounded::new(MGM1.into(), mgm_1_handler_mode_rx);
    /*
        let (mgm_0_spi_interface, mgm_1_spi_interface) =
            if let Some(sim_client) = opt_sim_client.as_mut() {
                sim_client
                    .add_reply_recipient(satrs_minisim::SimComponent::Mgm0Lis3Mdl, mgm_0_sim_reply_tx);
                sim_client
                    .add_reply_recipient(satrs_minisim::SimComponent::Mgm1Lis3Mdl, mgm_1_sim_reply_tx);
                (
                    SpiSimInterfaceWrapper::Sim(SpiSimInterface {
                        sim_request_tx: sim_request_tx.clone(),
                        sim_reply_rx: mgm_0_sim_reply_rx,
                    }),
                    SpiSimInterfaceWrapper::Sim(SpiSimInterface {
                        sim_request_tx: sim_request_tx.clone(),
                        sim_reply_rx: mgm_1_sim_reply_rx,
                    }),
                )
            } else {
                (
                    SpiSimInterfaceWrapper::Dummy(SpiDummyInterface::default()),
                    SpiSimInterfaceWrapper::Dummy(SpiDummyInterface::default()),
                )
            };
        let mut mgm_0_handler = MgmHandlerLis3Mdl::new(
            MGM0,
            "MGM_0",
            mgm_0_mode_node,
            mgm_0_handler_composite_rx,
            pus_hk_reply_tx.clone(),
            switch_helper.clone(),
            tm_sender.clone(),
            mgm_0_spi_interface,
            shared_mgm_0_set,
        );
        let mut mgm_1_handler = MgmHandlerLis3Mdl::new(
            MGM1,
            "MGM_1",
            mgm_1_mode_node,
            mgm_1_handler_composite_rx,
            pus_hk_reply_tx.clone(),
            switch_helper.clone(),
            tm_sender.clone(),
            mgm_1_spi_interface,
            shared_mgm_1_set,
        );
        // Connect PUS service to device handlers.
        connect_mode_nodes(
            &mut pus_stack.mode_srv,
            mgm_0_handler_mode_tx,
            &mut mgm_0_handler,
            pus_mode_reply_tx.clone(),
        );
        connect_mode_nodes(
            &mut pus_stack.mode_srv,
            mgm_1_handler_mode_tx,
            &mut mgm_1_handler,
            pus_mode_reply_tx.clone(),
        );
    */

    let pcdu_serial_interface = if let Some(sim_client) = opt_sim_client.as_mut() {
        sim_client.add_reply_recipient(satrs_minisim::SimComponent::Pcdu, pcdu_sim_reply_tx);
        SerialSimInterfaceWrapper::Sim(SerialInterfaceToSim::new(
            sim_request_tx.clone(),
            pcdu_sim_reply_rx,
        ))
    } else {
        SerialSimInterfaceWrapper::Dummy(SerialInterfaceDummy::default())
    };
    let pcdu_mode_node = ModeRequestHandlerMpscBounded::new(PCDU.into(), pcdu_handler_mode_rx);
    let mut pcdu_handler = PcduHandler::new(
        PCDU,
        "PCDU",
        pcdu_mode_node,
        pcdu_handler_composite_rx,
        pus_hk_reply_tx,
        switch_request_rx,
        tm_sink_tx.clone(),
        pcdu_serial_interface,
        shared_switch_set,
    );
    /*
    connect_mode_nodes(
        &mut pus_stack.mode_srv,
        pcdu_handler_mode_tx.clone(),
        &mut pcdu_handler,
        pus_mode_reply_tx,
    );
    */

    // The PCDU is a critical component which should be in normal mode immediately.
    pcdu_handler_mode_tx
        .send(GenericMessage::new(
            MessageMetadata::new(0, NO_SENDER),
            ModeRequest::SetMode {
                mode_and_submode: ModeAndSubmode::new(DeviceMode::Normal as Mode, 0),
                forced: false,
            },
        ))
        .expect("sending initial mode request failed");

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
            //mgm_0_handler.periodic_operation();
            //mgm_1_handler.periodic_operation();
            thread::sleep(Duration::from_millis(FREQ_MS_AOCS));
        })
        .unwrap();

    info!("Starting EPS thread");
    let jh_eps = thread::Builder::new()
        .name("sat-rs eps".to_string())
        .spawn(move || loop {
            // TODO: We should introduce something like a fixed timeslot helper to allow a more
            // declarative API. It would also be very useful for the AOCS task.
            //
            // TODO: The fixed timeslot handler exists.. use it.
            pcdu_handler.periodic_operation(crate::eps::pcdu::OpCode::RegularOp);
            thread::sleep(Duration::from_millis(50));
            pcdu_handler.periodic_operation(crate::eps::pcdu::OpCode::PollAndRecvReplies);
            thread::sleep(Duration::from_millis(50));
            pcdu_handler.periodic_operation(crate::eps::pcdu::OpCode::PollAndRecvReplies);
            thread::sleep(Duration::from_millis(300));
        })
        .unwrap();

    info!("Starting PUS handler thread");
    let jh_pus_handler = thread::Builder::new()
        .name("sat-rs pus".to_string())
        .spawn(move || loop {
            //event_handler.periodic_operation();
            //pus_stack.periodic_operation();
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
    jh_eps.join().expect("Joining EPS thread failed");
    jh_pus_handler
        .join()
        .expect("Joining PUS handler thread failed");
}

pub fn update_time(time_provider: &mut CdsTime, timestamp: &mut [u8]) {
    time_provider
        .update_from_now()
        .expect("Could not get current time");
    time_provider
        .write_to_bytes(timestamp)
        .expect("Writing timestamp failed");
}
