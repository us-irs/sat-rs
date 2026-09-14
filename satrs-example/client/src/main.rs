use anyhow::bail;
use arbitrary_int::u11;
use clap::Parser as _;
use satrs_example::config::{OBSW_SERVER_ADDR, SERVER_PORT};
use satrs_minisim::{
    SerializableSimMsgPayload, SimComponent, SimCtrlReply, SimCtrlRequest, SimMessageProvider,
    SimReply, SimRequest, acs::MgmRequestLis3Mdl, acs::SpiFaultMode, udp::SIM_CTRL_PORT,
};
use spacepackets::{CcsdsPacketIdAndPsc, SpacePacketHeader};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};
use types::{Apid, MessageType, TcHeader, acs::mgm::request::HkRequest};

#[derive(clap::Parser)]
pub struct Cli {
    #[arg(short, long)]
    ping: bool,
    #[arg(short, long)]
    test_event: bool,

    #[command(subcommand)]
    commands: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    Mgm0(MgmArgs),
    Mgm1(MgmArgs),
    MgmAssy(MgmAssemblyArgs),
    AcsSubsystem(SubsystemArgs),
    /// Inject a fault directly into the simulator, bypassing the OBSW entirely.
    ///
    /// This talks straight to minisim's own control port (the same one the OBSW's internal sim
    /// client uses), since fault injection is a test/simulation-environment capability, not
    /// something the real flight software could ever ask for.
    SimFault(SimFaultArgs),
}

impl Commands {
    /// The OBSW component this command is addressed to, if any. `SimFault` has none, since it
    /// never goes through the OBSW.
    #[inline]
    pub fn target_id(&self) -> Option<types::ComponentId> {
        match self {
            Commands::Mgm0(_mgm_args) => Some(types::ComponentId::AcsMgm0),
            Commands::Mgm1(_mgm_args) => Some(types::ComponentId::AcsMgm1),
            Commands::MgmAssy(_mgm_assembly_args) => Some(types::ComponentId::AcsMgmAssembly),
            Commands::AcsSubsystem(_subsystem_args) => Some(types::ComponentId::AcsSubsystem),
            Commands::SimFault(_) => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, clap::Parser)]
struct SimFaultArgs {
    /// Fault mode to inject on the simulated MGM0 SPI bus.
    #[arg(value_enum)]
    mode: SpiFaultModeSelect,
}

// MGM1 is not selectable here: MgmRequestLis3Mdl::TARGET is hardcoded to Mgm0Lis3Mdl, so a
// request built from this payload always reaches the MGM0 model regardless of intent. This
// appears to be a pre-existing minisim limitation, not something specific to fault injection.
#[derive(Debug, PartialEq, Eq, Clone, Copy, clap::ValueEnum)]
enum SpiFaultModeSelect {
    None,
    AllZeros,
    AllOnes,
}

impl From<SpiFaultModeSelect> for SpiFaultMode {
    fn from(mode: SpiFaultModeSelect) -> Self {
        match mode {
            SpiFaultModeSelect::None => SpiFaultMode::None,
            SpiFaultModeSelect::AllZeros => SpiFaultMode::AllZeros,
            SpiFaultModeSelect::AllOnes => SpiFaultMode::AllOnes,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, clap::Parser)]
struct MgmArgs {
    #[arg(short, long)]
    ping: bool,
    #[arg(long)]
    request_hk: bool,
    #[arg(short, long)]
    mode: Option<DeviceModeSelect>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, clap::Parser)]
struct MgmAssemblyArgs {
    #[arg(short, long)]
    ping: bool,
    #[arg(short, long)]
    mode: Option<AssemblyModeSelect>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, clap::Parser)]
struct SubsystemArgs {
    #[arg(short, long)]
    ping: bool,
    #[arg(short, long)]
    mode: Option<SubsystemModeSelect>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, clap::ValueEnum)]
pub enum DeviceModeSelect {
    Off,
    Normal,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, clap::ValueEnum)]
pub enum AssemblyModeSelect {
    NoModeKeeping,
    Off,
    Normal,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, clap::ValueEnum)]
pub enum SubsystemModeSelect {
    Off,
    Safe,
}

fn setup_logger(level: log::LevelFilter) -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339_seconds(SystemTime::now()),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(level)
        .chain(std::io::stdout())
        .chain(fern::log_file("output.log")?)
        .apply()?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    setup_logger(log::LevelFilter::Debug).unwrap();
    let kill_signal = Arc::new(AtomicBool::new(false));
    let ctrl_kill_signal = kill_signal.clone();
    ctrlc::set_handler(move || ctrl_kill_signal.store(true, Ordering::Relaxed)).unwrap();
    let cli = Cli::parse();

    let addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let client = UdpSocket::bind("127.0.0.1:7302").expect("Connecting to UDP server failed");
    client.set_nonblocking(true)?;
    client.set_read_timeout(Some(Duration::from_millis(200)))?;

    if cli.ping {
        let request = types::ccsds::CcsdsTcPacketOwned::new_with_request(
            SpacePacketHeader::new_from_apid(u11::new(Apid::Tmtc as u16)),
            TcHeader::new(types::ComponentId::Controller, types::MessageType::Ping),
            types::control::request::Request::Ping,
        );
        let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
        log::info!("sending ping request with TC ID {:#010x}", sent_tc_id.raw());
        let request_packet = request.to_vec();
        client.send_to(&request_packet, addr).unwrap();
    }
    if cli.test_event {
        let request = types::ccsds::CcsdsTcPacketOwned::new_with_request(
            SpacePacketHeader::new_from_apid(u11::new(Apid::Tmtc as u16)),
            TcHeader::new(types::ComponentId::Controller, types::MessageType::Event),
            types::control::request::Request::TestEvent,
        );
        let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
        log::info!(
            "sending event request with TC ID {:#010x}",
            sent_tc_id.raw()
        );
        let request_packet = request.to_vec();
        client.send_to(&request_packet, addr).unwrap();
    }
    if let Some(cmd) = cli.commands {
        let target_id = cmd.target_id();
        match cmd {
            Commands::Mgm0(args) | Commands::Mgm1(args) => {
                let target_id = target_id.expect("mgm commands always have a target id");
                if args.ping {
                    let request = types::ccsds::CcsdsTcPacketOwned::new_with_request(
                        SpacePacketHeader::new_from_apid(u11::new(Apid::Acs as u16)),
                        TcHeader::new(target_id, types::MessageType::Ping),
                        types::acs::mgm::request::Request::Ping,
                    );
                    let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
                    log::info!(
                        "sending {:?} ping request with TC ID {:#010x}",
                        target_id,
                        sent_tc_id.raw()
                    );
                    let request_packet = request.to_vec();
                    client.send_to(&request_packet, addr).unwrap();
                }
                if args.request_hk {
                    let request = types::ccsds::CcsdsTcPacketOwned::new_with_request(
                        SpacePacketHeader::new_from_apid(u11::new(Apid::Acs as u16)),
                        TcHeader::new(target_id, types::MessageType::Hk),
                        types::acs::mgm::request::Request::Hk(HkRequest {
                            id: types::acs::mgm::request::HkId::Sensor,
                            req_type: types::HkRequestType::OneShot,
                        }),
                    );
                    let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
                    log::info!(
                        "sending {:?} HK request with TC ID {:#010x}",
                        target_id,
                        sent_tc_id.raw()
                    );
                    let request_packet = request.to_vec();
                    client.send_to(&request_packet, addr).unwrap();
                }
                if let Some(mode) = args.mode {
                    let dev_mode = match mode {
                        DeviceModeSelect::Off => types::DeviceMode::Off,
                        DeviceModeSelect::Normal => types::DeviceMode::Normal,
                    };

                    let request = types::ccsds::CcsdsTcPacketOwned::new_with_request(
                        SpacePacketHeader::new_from_apid(u11::new(Apid::Acs as u16)),
                        TcHeader::new(target_id, types::MessageType::Mode),
                        types::acs::mgm::request::Request::Mode(
                            types::acs::mgm::request::ModeRequest::SetMode(dev_mode),
                        ),
                    );
                    let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
                    log::info!(
                        "sending {:?} HK request with TC ID {:#010x}",
                        target_id,
                        sent_tc_id.raw()
                    );
                    let request_packet = request.to_vec();
                    client.send_to(&request_packet, addr).unwrap();
                }
            }
            Commands::MgmAssy(mgm_assembly_args) => {
                let target_id = target_id.expect("mgm assembly command always has a target id");
                if mgm_assembly_args.ping {
                    let request = types::ccsds::CcsdsTcPacketOwned::new_with_request(
                        SpacePacketHeader::new_from_apid(u11::new(Apid::Acs as u16)),
                        TcHeader::new(target_id, types::MessageType::Ping),
                        types::acs::mgm::request::Request::Ping,
                    );
                    let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
                    log::info!(
                        "sending {:?} ping request with TC ID {:#010x}",
                        target_id,
                        sent_tc_id.raw()
                    );
                    let request_packet = request.to_vec();
                    client.send_to(&request_packet, addr).unwrap();
                }
                if let Some(mode) = mgm_assembly_args.mode {
                    let assembly_mode = match mode {
                        AssemblyModeSelect::NoModeKeeping => {
                            types::acs::mgm_assembly::Mode::NoModeKeeping
                        }
                        AssemblyModeSelect::Off => {
                            types::acs::mgm_assembly::Mode::Device(types::DeviceMode::Off)
                        }
                        AssemblyModeSelect::Normal => {
                            types::acs::mgm_assembly::Mode::Device(types::DeviceMode::Normal)
                        }
                    };

                    let request = types::ccsds::CcsdsTcPacketOwned::new_with_request(
                        SpacePacketHeader::new_from_apid(u11::new(Apid::Acs as u16)),
                        TcHeader::new(target_id, types::MessageType::Mode),
                        types::acs::mgm_assembly::request::Request::Mode(
                            types::acs::mgm_assembly::request::ModeRequest::SetMode(assembly_mode),
                        ),
                    );
                    let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
                    log::info!(
                        "sending {:?} HK request with TC ID {:#010x}",
                        target_id,
                        sent_tc_id.raw()
                    );
                    let request_packet = request.to_vec();
                    client.send_to(&request_packet, addr).unwrap();
                }
            }
            Commands::AcsSubsystem(subsystem_args) => {
                let target_id = target_id.expect("acs subsystem command always has a target id");
                if subsystem_args.ping {
                    let request = types::ccsds::CcsdsTcPacketOwned::new_with_request(
                        SpacePacketHeader::new_from_apid(u11::new(Apid::Acs as u16)),
                        TcHeader::new(target_id, types::MessageType::Ping),
                        types::acs::subsystem::request::Request::Ping,
                    );
                    let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
                    log::info!(
                        "sending {:?} ping request with TC ID {:#010x}",
                        target_id,
                        sent_tc_id.raw()
                    );
                    let request_packet = request.to_vec();
                    client.send_to(&request_packet, addr).unwrap();
                }
                if let Some(mode) = subsystem_args.mode {
                    let subsystem_mode = match mode {
                        SubsystemModeSelect::Off => types::acs::subsystem::Mode::Off,
                        SubsystemModeSelect::Safe => types::acs::subsystem::Mode::Safe,
                    };

                    let request = types::ccsds::CcsdsTcPacketOwned::new_with_request(
                        SpacePacketHeader::new_from_apid(u11::new(Apid::Acs as u16)),
                        TcHeader::new(target_id, types::MessageType::Mode),
                        types::acs::subsystem::request::Request::Mode(
                            types::acs::subsystem::request::ModeRequest::SetMode(subsystem_mode),
                        ),
                    );
                    let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
                    log::info!(
                        "sending {:?} mode request with TC ID {:#010x}",
                        target_id,
                        sent_tc_id.raw()
                    );
                    let request_packet = request.to_vec();
                    client.send_to(&request_packet, addr).unwrap();
                }
            }
            Commands::SimFault(sim_fault_args) => {
                inject_sim_fault(sim_fault_args.mode.into())?;
            }
        }
    }

    let mut recv_buf: Box<[u8; 2048]> = Box::new([0; 2048]);
    log::info!("entering listening loop");
    loop {
        if kill_signal.load(std::sync::atomic::Ordering::Relaxed) {
            log::info!("received kill signal, exiting");
            break;
        }
        match client.recv(recv_buf.as_mut_slice()) {
            Ok(received_bytes) => handle_raw_tm_packet(&recv_buf.as_slice()[0..received_bytes])?,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut
                {
                    continue;
                }
                log::warn!("UDP reception error: {}", e)
            }
        }
    }
    Ok(())
}

/// Injects the given SPI fault mode directly into minisim's MGM0 model, bypassing the OBSW.
///
/// Confirms the simulator is actually reachable first (same ping/pong check the OBSW's own
/// internal sim client does, see `SimClientUdp::attempt_connection`), since a fire-and-forget
/// UDP send would otherwise silently do nothing if minisim is not running.
fn inject_sim_fault(mode: SpiFaultMode) -> anyhow::Result<()> {
    let sim_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), SIM_CTRL_PORT);
    let sim_socket = UdpSocket::bind("127.0.0.1:0")?;
    sim_socket.set_read_timeout(Some(Duration::from_millis(200)))?;

    let mut reply_buf = [0u8; 4096];
    let ping = SimRequest::new_with_epoch_time(SimCtrlRequest::Ping);
    sim_socket.send_to(&serde_json::to_vec(&ping)?, sim_addr)?;
    match sim_socket.recv(&mut reply_buf) {
        Ok(len) => {
            let reply: SimReply = serde_json::from_slice(&reply_buf[..len])?;
            if reply.component() != SimComponent::SimCtrl {
                bail!("unexpected reply while checking minisim connectivity: {reply:?}");
            }
            match SimCtrlReply::from_sim_message(&reply).expect("invalid SIM reply") {
                SimCtrlReply::Pong => {}
                SimCtrlReply::InvalidRequest(e) => {
                    bail!("minisim rejected connectivity ping: {e:?}")
                }
            }
        }
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
            bail!("minisim not reachable at {sim_addr} (ping timed out) - is it running?");
        }
        Err(e) => return Err(e.into()),
    }

    let request = SimRequest::new_with_epoch_time(MgmRequestLis3Mdl::SetSpiFault(mode));
    sim_socket.send_to(&serde_json::to_vec(&request)?, sim_addr)?;
    log::info!("injected SPI fault mode {mode:?} into minisim MGM0");
    Ok(())
}

fn handle_raw_tm_packet(data: &[u8]) -> anyhow::Result<()> {
    match spacepackets::CcsdsPacketReader::new_with_checksum(data) {
        Ok(packet) => {
            let tm_header_result = postcard::take_from_bytes::<types::TmHeader>(packet.user_data());
            if let Err(e) = tm_header_result {
                bail!("Failed to deserialize TM header: {}", e);
            }
            let (tm_header, remainder) = tm_header_result.unwrap();
            if let Some(tc_id) = tm_header.tc_id {
                log::info!(
                    "Received TM with APID {} and from sender {:?} for TC ID {:#010x}",
                    packet.apid(),
                    tm_header.sender_id,
                    tc_id.raw()
                );
            } else {
                log::info!(
                    "Received unsolicited TM with APID {} and from sender {:?}",
                    packet.apid(),
                    tm_header.sender_id,
                );
            }
            if tm_header.message_type == MessageType::Event {
                let response = postcard::from_bytes::<types::Event>(remainder);
                log::info!(
                    "Received event from {:?}: {:?}",
                    tm_header.sender_id,
                    response.unwrap()
                );
                return Ok(());
            }
            match tm_header.sender_id {
                types::ComponentId::EpsPcdu => {
                    let response =
                        postcard::from_bytes::<types::pcdu::response::Response>(remainder);
                    log::info!("Received response from PCDU: {:?}", response.unwrap());
                }
                types::ComponentId::Controller => {
                    let response =
                        postcard::from_bytes::<types::control::response::Response>(remainder);
                    log::info!("Received response from controller: {:?}", response.unwrap());
                }
                types::ComponentId::AcsMgmAssembly => {
                    let response = postcard::from_bytes::<
                        types::acs::mgm_assembly::response::Response,
                    >(remainder);
                    log::info!(
                        "Received response from MGM Assembly: {:?}",
                        response.unwrap()
                    );
                }
                types::ComponentId::AcsMgm0 => {
                    let response =
                        postcard::from_bytes::<types::acs::mgm::response::Response>(remainder);
                    log::info!("Received response from MGM0: {:?}", response.unwrap());
                }
                types::ComponentId::AcsMgm1 => {
                    let response =
                        postcard::from_bytes::<types::acs::mgm::response::Response>(remainder);
                    log::info!("Received response from MGM1: {:?}", response.unwrap());
                }
                types::ComponentId::AcsSubsystem => {
                    let response = postcard::from_bytes::<types::acs::subsystem::response::Response>(
                        remainder,
                    );
                    log::info!(
                        "Received response from ACS subsystem: {:?}",
                        response.unwrap()
                    );
                }
                types::ComponentId::EpsSubsystem => todo!(),
                types::ComponentId::UdpServer => todo!(),
                types::ComponentId::TcpServer => todo!(),
                types::ComponentId::Ground => todo!(),
                types::ComponentId::EventManager => {}
                types::ComponentId::AcsController => todo!(),
                types::ComponentId::AcsMgt => todo!(),
            }
        }
        Err(_) => todo!(),
    }
    Ok(())
}
