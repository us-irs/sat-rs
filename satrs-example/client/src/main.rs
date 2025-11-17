use anyhow::bail;
use arbitrary_int::u11;
use clap::Parser as _;
use models::{Apid, MessageType, TcHeader};
use satrs_example::config::{OBSW_SERVER_ADDR, SERVER_PORT};
use spacepackets::{CcsdsPacketIdAndPsc, SpacePacketHeader};
use std::{
    net::{IpAddr, SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

#[derive(clap::Parser)]
pub struct Cli {
    #[arg(short, long)]
    ping: bool,
    #[arg(short, long)]
    test_event: bool,
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
        let request = models::ccsds::CcsdsTcPacketOwned::new_with_request(
            SpacePacketHeader::new_from_apid(u11::new(Apid::Tmtc as u16)),
            TcHeader::new(models::ComponentId::Controller, models::MessageType::Ping),
            models::control::request::Request::Ping,
        );
        let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
        log::info!("sending ping request with TC ID {:#010x}", sent_tc_id.raw());
        let request_packet = request.to_vec();
        client.send_to(&request_packet, addr).unwrap();
    }
    if cli.test_event {
        let request = models::ccsds::CcsdsTcPacketOwned::new_with_request(
            SpacePacketHeader::new_from_apid(u11::new(Apid::Tmtc as u16)),
            TcHeader::new(models::ComponentId::Controller, models::MessageType::Event),
            models::control::request::Request::TestEvent,
        );
        let sent_tc_id = CcsdsPacketIdAndPsc::new_from_ccsds_packet(&request.sp_header);
        log::info!(
            "sending event request with TC ID {:#010x}",
            sent_tc_id.raw()
        );
        let request_packet = request.to_vec();
        client.send_to(&request_packet, addr).unwrap();
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

fn handle_raw_tm_packet(data: &[u8]) -> anyhow::Result<()> {
    match spacepackets::CcsdsPacketReader::new_with_checksum(data) {
        Ok(packet) => {
            //let (tm_header, response, remainder) = unpack_tm_header_and_response(&packet)?;
            let tm_header_result =
                postcard::take_from_bytes::<models::TmHeader>(packet.user_data());
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
                let response = postcard::from_bytes::<models::Event>(remainder);
                log::info!(
                    "Received event from {:?}: {:?}",
                    tm_header.sender_id,
                    response.unwrap()
                );
                return Ok(());
            }
            match tm_header.sender_id {
                models::ComponentId::EpsPcdu => {
                    let response =
                        postcard::from_bytes::<models::pcdu::response::Response>(remainder);
                    log::info!("Received response from PCDU: {:?}", response.unwrap());
                }
                models::ComponentId::Controller => {
                    let response =
                        postcard::from_bytes::<models::control::response::Response>(remainder);
                    log::info!("Received response from controller: {:?}", response.unwrap());
                }
                models::ComponentId::AcsSubsystem => todo!(),
                models::ComponentId::AcsMgmAssembly => todo!(),
                models::ComponentId::AcsMgm0 => todo!(),
                models::ComponentId::AcsMgm1 => todo!(),
                models::ComponentId::EpsSubsystem => todo!(),
                models::ComponentId::UdpServer => todo!(),
                models::ComponentId::TcpServer => todo!(),
                models::ComponentId::Ground => todo!(),
                models::ComponentId::EventManager => {}
            }
        }
        Err(_) => todo!(),
    }
    Ok(())
}
