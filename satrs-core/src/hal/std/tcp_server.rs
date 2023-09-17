//! Generic TCP TMTC servers with different TMTC format flavours.
use alloc::vec;
use alloc::{boxed::Box, vec::Vec};
use core::time::Duration;
use socket2::{Domain, Socket, Type};
use std::io::Read;
use std::net::TcpListener;
use std::net::{SocketAddr, TcpStream};
use std::thread;

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

pub trait TcpTcHandler<TmError, TcError> {
    fn handle_tc_parsing(
        &mut self,
        tc_buffer: &mut [u8],
        tc_receiver: &mut dyn ReceivesTc<Error = TcError>,
        conn_result: &mut ConnectionResult,
        current_write_idx: usize,
        next_write_idx: &mut usize,
    ) -> Result<(), TcpTmtcError<TmError, TcError>>;
}
pub trait TcpTmHandler<TmError, TcError> {
    fn handle_tm_sending(
        &mut self,
        tm_buffer: &mut [u8],
        tm_source: &mut dyn TmPacketSource<Error = TmError>,
        conn_result: &mut ConnectionResult,
        stream: &mut TcpStream,
    ) -> Result<bool, TcpTmtcError<TmError, TcError>>;
}

pub struct TcpTmtcGenericServer<
    TmError,
    TcError,
    TmHandler: TcpTmHandler<TmError, TcError>,
    TcHandler: TcpTcHandler<TmError, TcError>,
> {
    base: TcpTmtcServerBase<TmError, TcError>,
    tc_handler: TcHandler,
    tm_handler: TmHandler,
}

impl<
        TmError: 'static,
        TcError: 'static,
        TmHandler: TcpTmHandler<TmError, TcError>,
        TcHandler: TcpTcHandler<TmError, TcError>,
    > TcpTmtcGenericServer<TmError, TcError, TmHandler, TcHandler>
{
    /// Create a new TMTC server which exchanges TMTC packets encoded with
    /// [COBS protocol](https://en.wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing).
    ///
    /// ## Parameter
    ///
    /// * `cfg` - Configuration of the server.
    /// * `tm_source` - Generic TM source used by the server to pull telemetry packets which are
    ///     then sent back to the client.
    /// * `tc_receiver` - Any received telecommand which was decoded successfully will be forwarded
    ///     to this TC receiver.
    pub fn new(
        cfg: ServerConfig,
        tc_handler: TcHandler,
        tm_handler: TmHandler,
        tm_source: Box<dyn TmPacketSource<Error = TmError> + Send>,
        tc_receiver: Box<dyn ReceivesTc<Error = TcError> + Send>,
    ) -> Result<TcpTmtcGenericServer<TmError, TcError, TmHandler, TcHandler>, std::io::Error> {
        Ok(Self {
            base: TcpTmtcServerBase::new(cfg, tm_source, tc_receiver)?,
            tc_handler,
            tm_handler,
        })
    }

    /// Retrieve the internal [TcpListener] class.
    pub fn listener(&mut self) -> &mut TcpListener {
        self.base.listener()
    }

    /// Can be used to retrieve the local assigned address of the TCP server. This is especially
    /// useful if using the port number 0 for OS auto-assignment.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.base.local_addr()
    }

    /// This call is used to handle the next connection to a client. Right now, it performs
    /// the following steps:
    ///
    /// 1. It calls the [std::net::TcpListener::accept] method internally using the blocking API
    ///    until a client connects.
    /// 2. It reads all the telecommands from the client, which are expected to be COBS
    ///    encoded packets.
    /// 3. After reading and parsing all telecommands, it sends back all telemetry it can retrieve
    ///    from the user specified [TmPacketSource] back to the client.
    pub fn handle_next_connection(
        &mut self,
    ) -> Result<ConnectionResult, TcpTmtcError<TmError, TcError>> {
        let mut connection_result = ConnectionResult::default();
        let mut current_write_idx;
        let mut next_write_idx = 0;
        let (mut stream, addr) = self.base.listener.accept()?;
        stream.set_nonblocking(true)?;
        connection_result.addr = Some(addr);
        current_write_idx = next_write_idx;
        loop {
            let read_result = stream.read(&mut self.base.tc_buffer[current_write_idx..]);
            match read_result {
                Ok(0) => {
                    // Connection closed by client. If any TC was read, parse for complete packets.
                    // After that, break the outer loop.
                    if current_write_idx > 0 {
                        self.tc_handler.handle_tc_parsing(
                            &mut self.base.tc_buffer,
                            self.base.tc_receiver.as_mut(),
                            &mut connection_result,
                            current_write_idx,
                            &mut next_write_idx,
                        )?;
                    }
                    break;
                }
                Ok(read_len) => {
                    current_write_idx += read_len;
                    // TC buffer is full, we must parse for complete packets now.
                    if current_write_idx == self.base.tc_buffer.capacity() {
                        self.tc_handler.handle_tc_parsing(
                            &mut self.base.tc_buffer,
                            self.base.tc_receiver.as_mut(),
                            &mut connection_result,
                            current_write_idx,
                            &mut next_write_idx,
                        )?;
                        current_write_idx = next_write_idx;
                    }
                }
                Err(e) => match e.kind() {
                    // As per [TcpStream::set_read_timeout] documentation, this should work for
                    // both UNIX and Windows.
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                        self.tc_handler.handle_tc_parsing(
                            &mut self.base.tc_buffer,
                            self.base.tc_receiver.as_mut(),
                            &mut connection_result,
                            current_write_idx,
                            &mut next_write_idx,
                        )?;
                        current_write_idx = next_write_idx;

                        if !self.tm_handler.handle_tm_sending(
                            &mut self.base.tm_buffer,
                            self.base.tm_source.as_mut(),
                            &mut connection_result,
                            &mut stream,
                        )? {
                            // No TC read, no TM was sent, but the client has not disconnected.
                            // Perform an inner delay to avoid burning CPU time.
                            thread::sleep(self.base.inner_loop_delay);
                        }
                    }
                    _ => {
                        return Err(TcpTmtcError::Io(e));
                    }
                },
            }
        }
        self.tm_handler.handle_tm_sending(
            &mut self.base.tm_buffer,
            self.base.tm_source.as_mut(),
            &mut connection_result,
            &mut stream,
        )?;
        Ok(connection_result)
    }
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
