//! Generic TCP TMTC servers with different TMTC format flavours.
use alloc::vec;
use alloc::{boxed::Box, vec::Vec};
use core::time::Duration;
use socket2::{Domain, Socket, Type};
use std::net::SocketAddr;
use std::net::TcpListener;

use crate::tmtc::{ReceivesTc, TmPacketSource};
use thiserror::Error;

// Re-export the TMTC in COBS server.
pub use crate::hal::std::tcp_with_cobs_server::{
    parse_buffer_for_cobs_encoded_packets, TcpTmtcInCobsServer,
};

#[derive(Error, Debug)]
pub enum TcpTmtcError<TmError, TcError> {
    #[error("TM retrieval error: {0}")]
    TmError(TmError),
    #[error("TC retrieval error: {0}")]
    TcError(TcError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of one connection attempt. Contains the client address if a connection was established,
/// in addition to the number of telecommands and telemetry packets exchanged.
#[derive(Debug, Default)]
pub struct ConnectionResult {
    pub addr: Option<SocketAddr>,
    pub num_received_tcs: u32,
    pub num_sent_tms: u32,
}

pub(crate) struct TcpTmtcServerBase<TcError, TmError> {
    pub(crate) listener: TcpListener,
    pub(crate) inner_loop_delay: Duration,
    pub(crate) tm_source: Box<dyn TmPacketSource<Error = TmError> + Send>,
    pub(crate) tm_buffer: Vec<u8>,
    pub(crate) tc_receiver: Box<dyn ReceivesTc<Error = TcError> + Send>,
    pub(crate) tc_buffer: Vec<u8>,
}

impl<TcError, TmError> TcpTmtcServerBase<TcError, TmError> {
    pub(crate) fn new(
        addr: &SocketAddr,
        inner_loop_delay: Duration,
        reuse_addr: bool,
        reuse_port: bool,
        tm_buffer_size: usize,
        tm_source: Box<dyn TmPacketSource<Error = TmError> + Send>,
        tc_buffer_size: usize,
        tc_receiver: Box<dyn ReceivesTc<Error = TcError> + Send>,
    ) -> Result<Self, std::io::Error> {
        // Create a TCP listener bound to two addresses.
        let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
        socket.set_reuse_address(reuse_addr)?;
        socket.set_reuse_port(reuse_port)?;
        let addr = (*addr).into();
        socket.bind(&addr)?;
        socket.listen(128)?;
        Ok(Self {
            listener: socket.into(),
            inner_loop_delay,
            tm_source,
            tm_buffer: vec![0; tm_buffer_size],
            tc_receiver,
            tc_buffer: vec![0; tc_buffer_size],
        })
    }

    pub(crate) fn listener(&mut self) -> &mut TcpListener {
        &mut self.listener
    }

    pub(crate) fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}
