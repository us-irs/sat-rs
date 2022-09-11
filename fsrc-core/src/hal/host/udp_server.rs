//! UDP server helper components
use crate::tmtc::ReceivesTc;
use std::boxed::Box;
use std::io::{Error, ErrorKind};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::vec;
use std::vec::Vec;

/// This TC server helper can be used to receive raw PUS telecommands thorough a UDP interface.
///
/// It caches all received telecomands into a vector. The maximum expected telecommand size should
/// be declared upfront. This avoids dynamic allocation during run-time. The user can specify a TC
/// receiver in form of a special trait object which implements [ReceivesTc]. Please note that the
/// receiver should copy out the received data if it the data is required past the
/// [ReceivesTc::pass_tc] call.
///
/// # Examples
///
/// The [fsrc-example crate](https://egit.irs.uni-stuttgart.de/rust/fsrc-launchpad/src/branch/obsw-client-example/fsrc-example) server code includes
/// [example code](https://egit.irs.uni-stuttgart.de/rust/fsrc-launchpad/src/branch/obsw-client-example/fsrc-example/src/bin/obsw/tmtc.rs)
/// on how to use this TC server. It uses the server to receive PUS telecommands on a specific port
/// and then forwards them to a generic CCSDS packet receiver.
pub struct UdpTcServer<E> {
    pub socket: UdpSocket,
    recv_buf: Vec<u8>,
    sender_addr: Option<SocketAddr>,
    tc_receiver: Box<dyn ReceivesTc<Error = E>>,
}

#[derive(Debug)]
pub enum ReceiveResult<E> {
    WouldBlock,
    IoError(Error),
    ReceiverError(E),
}

impl<E> From<Error> for ReceiveResult<E> {
    fn from(e: Error) -> Self {
        ReceiveResult::IoError(e)
    }
}

impl<E> UdpTcServer<E> {
    pub fn new<A: ToSocketAddrs>(
        addr: A,
        max_recv_size: usize,
        tc_receiver: Box<dyn ReceivesTc<Error = E>>,
    ) -> Result<Self, Error> {
        let server = Self {
            socket: UdpSocket::bind(addr)?,
            recv_buf: vec![0; max_recv_size],
            sender_addr: None,
            tc_receiver,
        };
        server.socket.set_nonblocking(true)?;
        Ok(server)
    }

    pub fn try_recv_tc(&mut self) -> Result<(usize, SocketAddr), ReceiveResult<E>> {
        let res = match self.socket.recv_from(&mut self.recv_buf) {
            Ok(res) => res,
            Err(e) => {
                return if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut {
                    Err(ReceiveResult::WouldBlock)
                } else {
                    Err(e.into())
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
