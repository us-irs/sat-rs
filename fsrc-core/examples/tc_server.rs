extern crate core;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::thread;

fn main() {
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 7301);
    let socket = UdpSocket::bind(&server_addr.clone()).expect("Error opening UDP socket");
    let mut recv_buf = [0; 1024];
    let jh = thread::spawn(move || {
        let dummy_data = [1, 2, 3, 4];
        let client = UdpSocket::bind("127.0.0.1:7300").expect("Connecting to UDP server failed");
        client
            .send_to(&dummy_data, &server_addr)
            .expect(&*format!("Sending to {:?} failed", server_addr));
    });
    let (num_bytes, src) = socket.recv_from(&mut recv_buf).expect("UDP Receive error");
    println!(
        "Received {num_bytes} bytes from {src}: {:x?}",
        &recv_buf[0..num_bytes]
    );
    jh.join().expect("Joining thread failed");
}
