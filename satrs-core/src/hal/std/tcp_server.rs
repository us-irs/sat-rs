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

/// TCP configuration struct.
///
/// ## Parameters
///
/// * `addr` - Address of the TCP server.
/// * `inner_loop_delay` - If a client connects for a longer period, but no TC is received or
///     no TM needs to be sent, the TCP server will delay for the specified amount of time
///     to reduce CPU load.
/// * `tm_buffer_size` - Size of the TM buffer used to read TM from the [TmPacketSource] and
///     encoding of that data. This buffer should at large enough to hold the maximum expected
///     TM size in addition to the COBS encoding overhead. You can use
///     [cobs::max_encoding_length] to calculate this size.
/// * `tc_buffer_size` - Size of the TC buffer used to read encoded telecommands sent from
///     the client. It is recommended to make this buffer larger to allow reading multiple
///     consecutive packets as well, for example by using 4096 or 8192 byte. The buffer should
///     at the very least be large enough to hold the maximum expected telecommand size in
///     addition to its COBS encoding overhead. You can use [cobs::max_encoding_length] to
///     calculate this size.
/// * `reuse_addr` - Can be used to set the `SO_REUSEADDR` option on the raw socket. This is
///     especially useful if the address and port are static for the server. Set to false by
///     default.
/// * `reuse_port` - Can be used to set the `SO_REUSEPORT` option on the raw socket. This is
///     especially useful if the address and port are static for the server. Set to false by
///     default.
#[derive(Debug, Copy, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub inner_loop_delay: Duration,
    pub tm_buffer_size: usize,
    pub tc_buffer_size: usize,
    pub reuse_addr: bool,
    pub reuse_port: bool,
}

impl ServerConfig {
    pub fn new(
        addr: SocketAddr,
        inner_loop_delay: Duration,
        tm_buffer_size: usize,
        tc_buffer_size: usize,
    ) -> Self {
        Self {
            addr,
            inner_loop_delay,
            tm_buffer_size,
            tc_buffer_size,
            reuse_addr: false,
            reuse_port: false,
        }
    }
}

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

pub(crate) struct TcpTmtcServerBase<TmError, TcError> {
    pub(crate) listener: TcpListener,
    pub(crate) inner_loop_delay: Duration,
    pub(crate) tm_source: Box<dyn TmPacketSource<Error = TmError> + Send>,
    pub(crate) tm_buffer: Vec<u8>,
    pub(crate) tc_receiver: Box<dyn ReceivesTc<Error = TcError> + Send>,
    pub(crate) tc_buffer: Vec<u8>,
}

impl<TmError, TcError> TcpTmtcServerBase<TmError, TcError> {
    pub(crate) fn new(
        cfg: ServerConfig,
        tm_source: Box<dyn TmPacketSource<Error = TmError> + Send>,
        tc_receiver: Box<dyn ReceivesTc<Error = TcError> + Send>,
    ) -> Result<Self, std::io::Error> {
        // Create a TCP listener bound to two addresses.
        let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
        socket.set_reuse_address(cfg.reuse_addr)?;
        socket.set_reuse_port(cfg.reuse_port)?;
        let addr = (cfg.addr).into();
        socket.bind(&addr)?;
        socket.listen(128)?;
        Ok(Self {
            listener: socket.into(),
            inner_loop_delay: cfg.inner_loop_delay,
            tm_source,
            tm_buffer: vec![0; cfg.tm_buffer_size],
            tc_receiver,
            tc_buffer: vec![0; cfg.tc_buffer_size],
        })
    }

    pub(crate) fn listener(&mut self) -> &mut TcpListener {
        &mut self.listener
    }

    pub(crate) fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}
