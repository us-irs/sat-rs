use crate::tmtc::ReceivesTc;
use std::boxed::Box;
use std::io::ErrorKind;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::vec;
use std::vec::Vec;

pub struct UdpTcServer<E> {
    pub socket: UdpSocket,
    recv_buf: Vec<u8>,
    sender_addr: Option<SocketAddr>,
    tc_receiver: Box<dyn ReceivesTc<Error = E>>,
}

#[derive(Debug)]
pub enum ReceiveResult<E> {
    WouldBlock,
    OtherIoError(std::io::Error),
    ReceiverError(E),
}

impl<E> UdpTcServer<E> {
    pub fn new<A: ToSocketAddrs>(
        addr: A,
        max_recv_size: usize,
        tc_receiver: Box<dyn ReceivesTc<Error = E>>,
    ) -> Result<Self, std::io::Error> {
        let server = Self {
            socket: UdpSocket::bind(addr)?,
            recv_buf: vec![0; max_recv_size],
            sender_addr: None,
            tc_receiver,
        };
        server
            .socket
            .set_nonblocking(true)
            .expect("Setting server non blocking failed");
        Ok(server)
    }

    pub fn try_recv_tc(&mut self) -> Result<(usize, SocketAddr), ReceiveResult<E>> {
        let res = match self.socket.recv_from(&mut self.recv_buf) {
            Ok(res) => res,
            Err(e) => {
                if e.kind() != ErrorKind::WouldBlock {
                    return Err(ReceiveResult::WouldBlock);
                } else {
                    return Err(ReceiveResult::OtherIoError(e));
                }
            }
        };
        let (num_bytes, from) = res;
        self.sender_addr = Some(from);
        self.tc_receiver
            .pass_tc(&self.recv_buf[0..num_bytes])
            .map_err(|e| ReceiveResult::ReceiverError(e))?;
        Ok(res)
    }

    pub fn last_sender(&self) -> Option<SocketAddr> {
        self.sender_addr
    }
}
