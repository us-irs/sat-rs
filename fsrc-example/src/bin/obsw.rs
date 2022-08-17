use fsrc_example::{OBSW_SERVER_ADDR, SERVER_PORT};
use std::net::{IpAddr, SocketAddr, UdpSocket};

fn main() {
    let mut recv_buf = [0; 1024];
    let addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let socket = UdpSocket::bind(&addr).expect("Error opening UDP socket");
    loop {
        let (num_bytes, src) = socket.recv_from(&mut recv_buf).expect("UDP Receive error");
        println!(
            "Received TM with len {num_bytes} from {src}: {:x?}",
            &recv_buf[0..num_bytes]
        );
    }
}
