#![allow(unused_imports)]
use rmp_serde::{Deserializer, Serializer};
use satrs_minisim::eps::{SwitchMap, SwitchMapWrapper};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{SocketAddr, UdpSocket},
};

#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SwitchSet {
    pub valid: bool,
    pub switch_map: SwitchMap,
}

pub struct UdpServer {
    socket: UdpSocket,
    last_sender: Option<SocketAddr>,
}

impl Default for UdpServer {
    fn default() -> Self {
        UdpServer {
            socket: UdpSocket::bind("127.0.0.1:7301").expect("binding UDP socket failed"),
            last_sender: Default::default(),
        }
    }
}

impl UdpServer {
    pub fn send_back_reply(&self, reply: &[u8]) {
        self.socket
            .send_to(reply, self.last_sender.expect("last sender not set"))
            .expect("sending back failed");
    }
}

fn main() {
    let mut udp_server = UdpServer::default();

    loop {
        // Receives a single datagram message on the socket. If `buf` is too small to hold
        // the message, it will be cut off.
        let mut buf = [0; 4096];
        let (received, src) = udp_server
            .socket
            .recv_from(&mut buf)
            .expect("receive call failed");
        udp_server.last_sender = Some(src);
        println!("received {} bytes from {:?}", received, src);
        let switch_map_off = SwitchMapWrapper::default();
        let switch_set = SwitchSet {
            valid: true,
            switch_map: switch_map_off.0.clone(),
        };
        let switch_map_off_json =
            serde_json::to_string(&switch_set).expect("json serialization failed");
        println!("sending back reply: {}", switch_map_off_json);
        udp_server.send_back_reply(switch_map_off_json.as_bytes());
    }
}
