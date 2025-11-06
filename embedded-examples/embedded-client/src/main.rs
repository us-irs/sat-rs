use std::{
    fs::File,
    io::Read,
    path::Path,
    time::{Duration, SystemTime},
};

use clap::Parser;
use cobs::CobsDecoderOwned;
use satrs_stm32f3_disco_rtic::Request;
use spacepackets::{CcsdsPacketCreatorOwned, CcsdsPacketReader, SpHeader};
use tmtc_utils::transport::serial::PacketTransportSerialCobs;

#[derive(Parser, Debug)]
struct Cli {
    #[arg(short, long)]
    ping: bool,

    /// Set frequency in milliseconds.
    #[arg(short, long)]
    set_led_frequency: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
struct Config {
    interface: Interface,
}

#[derive(Debug, serde::Deserialize)]
struct Interface {
    serial_port: String,
}

fn setup_logger() -> Result<(), fern::InitError> {
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
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        .chain(fern::log_file("output.log")?)
        .apply()?;
    Ok(())
}

fn main() {
    setup_logger().expect("failed to initialize logger");
    println!("sat-rs embedded examples TMTC client");

    let cli = Cli::parse();
    let mut config_file =
        File::open(Path::new("config.toml")).expect("opening config.toml file failed");
    let mut toml_str = String::new();
    config_file
        .read_to_string(&mut toml_str)
        .expect("reading config.toml file failed");
    let config: Config = toml::from_str(&toml_str).expect("parsing config.toml file failed");
    println!("Connecting to serial port {}", config.interface.serial_port);

    let serial = serialport::new(config.interface.serial_port, 115200)
        .open()
        .expect("opening serial port failed");
    let mut transport = PacketTransportSerialCobs::new(serial, CobsDecoderOwned::new(1024));

    if cli.ping {
        let request = Request::Ping;
        let tc = create_stm32f3_tc(&request);
        log::info!(
            "Sending ping request with TC ID: {:#010x}",
            tc.ccsds_packet_id_and_psc().raw()
        );
        transport.send(&tc.to_vec()).unwrap();
    }

    if let Some(freq_ms) = cli.set_led_frequency {
        let request = Request::ChangeBlinkFrequency(Duration::from_millis(freq_ms as u64));
        let tc = create_stm32f3_tc(&request);
        log::info!(
            "Sending change blink frequency request {:?} with TC ID: {:#010x}",
            request,
            tc.ccsds_packet_id_and_psc().raw()
        );
        transport.send(&tc.to_vec()).unwrap();
    }

    log::info!("Waiting for response...");
    loop {
        transport
            .receive(|packet: &[u8]| {
                let reader = CcsdsPacketReader::new_with_checksum(packet);
                log::info!("Received packet: {:?}", reader);
            })
            .unwrap();
    }
}

fn create_stm32f3_tc(request: &Request) -> CcsdsPacketCreatorOwned {
    let req_raw = postcard::to_allocvec(&request).unwrap();
    let sp_header = SpHeader::new_from_apid(satrs_stm32f3_disco_rtic::APID);
    CcsdsPacketCreatorOwned::new_tc_with_checksum(sp_header, &req_raw).unwrap()
}
