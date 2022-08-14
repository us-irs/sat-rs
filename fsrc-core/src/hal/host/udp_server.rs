use crate::hal::host::udp_server::ReceiveResult::{IoError, ReceiverError};
use crate::tmtc::ReceivesTc;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::vec::Vec;

pub struct UdpTmtcServer<E> {
    socket: UdpSocket,
    recv_buf: Vec<u8>,
    tc_receiver: Box<dyn ReceivesTc<Error = E>>,
}

pub enum ReceiveResult<E> {
    IoError(std::io::Error),
    ReceiverError(E),
}

impl<E> UdpTmtcServer<E> {
    pub fn new<A: ToSocketAddrs>(
        addr: A,
        max_recv_size: usize,
        tc_receiver: Box<dyn ReceivesTc<Error = E>>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            socket: UdpSocket::bind(addr)?,
            recv_buf: Vec::with_capacity(max_recv_size),
            tc_receiver,
        })
    }

    pub fn recv_tc(&mut self) -> Result<(usize, SocketAddr), ReceiveResult<E>> {
        let res = self
            .socket
            .recv_from(&mut self.recv_buf)
            .map_err(|e| IoError(e))?;
        self.tc_receiver
            .pass_tc(&self.recv_buf[0..res.0])
            .map_err(|e| ReceiverError(e))?;
        Ok(res)
    }
}
