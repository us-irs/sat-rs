use satrs::pus::verification::RequestId;
use satrs::spacepackets::ecss::tc::PusTcCreator;
use satrs::spacepackets::ecss::tm::PusTmReader;
use satrs::spacepackets::ecss::WritablePusPacket as _;
use satrs::{spacepackets::ecss::PusPacket, spacepackets::SpHeader};
use satrs_example::config::{OBSW_SERVER_ADDR, SERVER_PORT};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

fn main() {
    let mut buf = [0; 32];
    let addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let pus_tc = PusTcCreator::new_simple(SpHeader::new_from_apid(0x02), 17, 1, &[], true);
    let client = UdpSocket::bind("127.0.0.1:7302").expect("Connecting to UDP server failed");
    let tc_req_id = RequestId::new(&pus_tc);
    println!("Packing and sending PUS ping command TC[17,1] with request ID {tc_req_id}");
    let size = pus_tc
        .write_to_bytes(&mut buf)
        .expect("Creating PUS TC failed");
    client
        .send_to(&buf[0..size], addr)
        .unwrap_or_else(|_| panic!("Sending to {addr:?} failed"));
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("Setting read timeout failed");
    loop {
        let res = client.recv(&mut buf);
        match res {
            Ok(_len) => {
                let pus_tm = PusTmReader::new(&buf, 7).expect("Parsing PUS TM failed");
                if pus_tm.service() == 17 && pus_tm.subservice() == 2 {
                    println!("Received PUS Ping Reply TM[17,2]")
                } else if pus_tm.service() == 1 {
                    if pus_tm.source_data().is_empty() {
                        println!("Invalid verification TM, no source data");
                    }
                    let src_data = pus_tm.source_data();
                    if src_data.len() < 4 {
                        println!("Invalid verification TM source data, less than 4 bytes")
                    }
                    let req_id = RequestId::from_bytes(src_data).unwrap();
                    if pus_tm.subservice() == 1 {
                        println!("Received TM[1,1] acceptance success for request ID {req_id}")
                    } else if pus_tm.subservice() == 2 {
                        println!("Received TM[1,2] acceptance failure for request ID {req_id}")
                    } else if pus_tm.subservice() == 3 {
                        println!("Received TM[1,3] start success for request ID {req_id}")
                    } else if pus_tm.subservice() == 4 {
                        println!("Received TM[1,2] start failure for request ID {req_id}")
                    } else if pus_tm.subservice() == 5 {
                        println!("Received TM[1,5] step success for request ID {req_id}")
                    } else if pus_tm.subservice() == 6 {
                        println!("Received TM[1,6] step failure for request ID {req_id}")
                    } else if pus_tm.subservice() == 7 {
                        println!("Received TM[1,7] completion success for request ID {req_id}")
                    } else if pus_tm.subservice() == 8 {
                        println!("Received TM[1,8] completion failure for request ID {req_id}");
                    }
                } else {
                    println!(
                        "Received TM[{}, {}] with {} bytes",
                        pus_tm.service(),
                        pus_tm.subservice(),
                        size
                    );
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                println!("No reply received for 2 seconds");
                break;
            }
            _ => {
                println!("UDP receive error {:?}", res.unwrap_err());
            }
        }
    }
}
