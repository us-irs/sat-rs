use fsrc_example::{OBSW_SERVER_ADDR, SERVER_PORT};
use spacepackets::tc::PusTc;
use spacepackets::SpHeader;
use std::net::{IpAddr, SocketAddr, UdpSocket};

fn main() {
    let mut buf = [0; 32];
    let addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let mut sph = SpHeader::tc(0x02, 0, 0).unwrap();
    let pus_tc = PusTc::new_simple(&mut sph, 17, 1, None, true);
    let client = UdpSocket::bind("127.0.0.1:7300").expect("Connecting to UDP server failed");
    let size = pus_tc.write_to(&mut buf).expect("Creating PUS TC failed");
    client
        .send_to(&buf[0..size], &addr)
        .expect(&*format!("Sending to {:?} failed", addr));
}
