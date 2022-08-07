use crate::tmtc::ReceivesTc;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::vec::Vec;

pub struct UdpTmtcServer {
    socket: UdpSocket,
    recv_buf: Vec<u8>,
    tc_receiver: Box<dyn ReceivesTc>,
}

impl UdpTmtcServer {
    pub fn new<A: ToSocketAddrs, E>(
        addr: A,
        max_recv_size: usize,
        tc_receiver: Box<dyn ReceivesTc>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            socket: UdpSocket::bind(addr)?,
            recv_buf: Vec::with_capacity(max_recv_size),
            tc_receiver,
        })
    }

    pub fn recv_tc(&mut self) -> Result<(usize, SocketAddr), std::io::Error> {
        let res = self.socket.recv_from(&mut self.recv_buf)?;
        self.tc_receiver.pass_tc(&self.recv_buf[0..res.0]);
        Ok(res)
    }
}
