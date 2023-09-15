use alloc::vec;
use alloc::{boxed::Box, vec::Vec};
use std::net::SocketAddr;
use std::net::{TcpListener, ToSocketAddrs};

use crate::tmtc::{ReceivesTc, TmPacketSource};
use core::fmt::Display;
use thiserror::Error;

// Re-export the TMTC in COBS server.
pub use crate::hal::host::tcp_with_cobs_server::TcpTmtcInCobsServer;

#[derive(Error, Debug)]
pub enum TcpTmtcError<TmError: Display, TcError: Display> {
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
    pub (crate) listener: TcpListener,
    pub (crate) tm_source: Box<dyn TmPacketSource<Error = TmError>>,
    pub (crate) tm_buffer: Vec<u8>,
    pub (crate) tc_receiver: Box<dyn ReceivesTc<Error = TcError>>,
    pub (crate) tc_buffer: Vec<u8>,
}

impl<TcError, TmError> TcpTmtcServerBase<TcError, TmError> {
    pub(crate) fn new<A: ToSocketAddrs>(
        addr: A,
        tm_buffer_size: usize,
        tm_source: Box<dyn TmPacketSource<Error = TmError>>,
        tc_buffer_size: usize,
        tc_receiver: Box<dyn ReceivesTc<Error = TcError>>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            listener: TcpListener::bind(addr)?,
            tm_source,
            tm_buffer: vec![0; tm_buffer_size],
            tc_receiver,
            tc_buffer: vec![0; tc_buffer_size],
        })
    }
}
