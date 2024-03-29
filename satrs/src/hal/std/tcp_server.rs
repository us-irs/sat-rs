//! Generic TCP TMTC servers with different TMTC format flavours.
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;
use socket2::{Domain, Socket, Type};
use std::io::Read;
use std::net::TcpListener;
use std::net::{SocketAddr, TcpStream};
use std::thread;

use crate::tmtc::{ReceivesTc, TmPacketSource};
use thiserror::Error;

// Re-export the TMTC in COBS server.
pub use crate::hal::std::tcp_cobs_server::{CobsTcParser, CobsTmSender, TcpTmtcInCobsServer};
pub use crate::hal::std::tcp_spacepackets_server::{
    SpacepacketsTcParser, SpacepacketsTmSender, TcpSpacepacketsServer,
};

/// Configuration struct for the generic TCP TMTC server
///
/// ## Parameters
///
/// * `addr` - Address of the TCP server.
/// * `inner_loop_delay` - If a client connects for a longer period, but no TC is received or
///     no TM needs to be sent, the TCP server will delay for the specified amount of time
///     to reduce CPU load.
/// * `tm_buffer_size` - Size of the TM buffer used to read TM from the [TmPacketSource] and
///     encoding of that data. This buffer should at large enough to hold the maximum expected
///     TM size read from the packet source.
/// * `tc_buffer_size` - Size of the TC buffer used to read encoded telecommands sent from
///     the client. It is recommended to make this buffer larger to allow reading multiple
///     consecutive packets as well, for example by using common buffer sizes like 4096 or 8192
///     byte. The buffer should at the very least be large enough to hold the maximum expected
///     telecommand size.
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

/// Generic parser abstraction for an object which can parse for telecommands given a raw
/// bytestream received from a TCP socket and send them to a generic [ReceivesTc] telecommand
/// receiver. This allows different encoding schemes for telecommands.
pub trait TcpTcParser<TmError, TcError> {
    fn handle_tc_parsing(
        &mut self,
        tc_buffer: &mut [u8],
        tc_receiver: &mut (impl ReceivesTc<Error = TcError> + ?Sized),
        conn_result: &mut ConnectionResult,
        current_write_idx: usize,
        next_write_idx: &mut usize,
    ) -> Result<(), TcpTmtcError<TmError, TcError>>;
}

/// Generic sender abstraction for an object which can pull telemetry from a given TM source
/// using a [TmPacketSource] and then send them back to a client using a given [TcpStream].
/// The concrete implementation can also perform any encoding steps which are necessary before
/// sending back the data to a client.
pub trait TcpTmSender<TmError, TcError> {
    fn handle_tm_sending(
        &mut self,
        tm_buffer: &mut [u8],
        tm_source: &mut (impl TmPacketSource<Error = TmError> + ?Sized),
        conn_result: &mut ConnectionResult,
        stream: &mut TcpStream,
    ) -> Result<bool, TcpTmtcError<TmError, TcError>>;
}

/// TCP TMTC server implementation for exchange of generic TMTC packets in a generic way which
/// stays agnostic to the encoding scheme and format used for both telecommands and telemetry.
///
/// This server implements a generic TMTC handling logic and allows modifying its behaviour
/// through the following 4 core abstractions:
///
/// 1. [TcpTcParser] to parse for telecommands from the raw bytestream received from a client.
/// 2. Parsed telecommands will be sent to the [ReceivesTc] telecommand receiver.
/// 3. [TcpTmSender] to send telemetry pulled from a TM source back to the client.
/// 4. [TmPacketSource] as a generic TM source used by the [TcpTmSender].
///
/// It is possible to specify custom abstractions to build a dedicated TCP TMTC server without
/// having to re-implement common logic.
///
/// Currently, this framework offers the following concrete implementations:
///
/// 1. [TcpTmtcInCobsServer] to exchange TMTC wrapped inside the COBS framing protocol.
pub struct TcpTmtcGenericServer<
    TmError,
    TcError,
    TmSource: TmPacketSource<Error = TmError>,
    TcReceiver: ReceivesTc<Error = TcError>,
    TmSender: TcpTmSender<TmError, TcError>,
    TcParser: TcpTcParser<TmError, TcError>,
> {
    pub(crate) listener: TcpListener,
    pub(crate) inner_loop_delay: Duration,
    pub(crate) tm_source: TmSource,
    pub(crate) tm_buffer: Vec<u8>,
    pub(crate) tc_receiver: TcReceiver,
    pub(crate) tc_buffer: Vec<u8>,
    tc_handler: TcParser,
    tm_handler: TmSender,
}

impl<
        TmError: 'static,
        TcError: 'static,
        TmSource: TmPacketSource<Error = TmError>,
        TcReceiver: ReceivesTc<Error = TcError>,
        TmSender: TcpTmSender<TmError, TcError>,
        TcParser: TcpTcParser<TmError, TcError>,
    > TcpTmtcGenericServer<TmError, TcError, TmSource, TcReceiver, TmSender, TcParser>
{
    /// Create a new generic TMTC server instance.
    ///
    /// ## Parameter
    ///
    /// * `cfg` - Configuration of the server.
    /// * `tc_parser` - Parser which extracts telecommands from the raw bytestream received from
    ///    the client.
    /// * `tm_sender` - Sends back telemetry to the client using the specified TM source.
    /// * `tm_source` - Generic TM source used by the server to pull telemetry packets which are
    ///     then sent back to the client.
    /// * `tc_receiver` - Any received telecommand which was decoded successfully will be forwarded
    ///     to this TC receiver.
    pub fn new(
        cfg: ServerConfig,
        tc_parser: TcParser,
        tm_sender: TmSender,
        tm_source: TmSource,
        tc_receiver: TcReceiver,
    ) -> Result<Self, std::io::Error> {
        // Create a TCP listener bound to two addresses.
        let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
        socket.set_reuse_address(cfg.reuse_addr)?;
        #[cfg(unix)]
        socket.set_reuse_port(cfg.reuse_port)?;
        let addr = (cfg.addr).into();
        socket.bind(&addr)?;
        socket.listen(128)?;
        Ok(Self {
            tc_handler: tc_parser,
            tm_handler: tm_sender,
            listener: socket.into(),
            inner_loop_delay: cfg.inner_loop_delay,
            tm_source,
            tm_buffer: vec![0; cfg.tm_buffer_size],
            tc_receiver,
            tc_buffer: vec![0; cfg.tc_buffer_size],
        })
    }

    /// Retrieve the internal [TcpListener] class.
    pub fn listener(&mut self) -> &mut TcpListener {
        &mut self.listener
    }

    /// Can be used to retrieve the local assigned address of the TCP server. This is especially
    /// useful if using the port number 0 for OS auto-assignment.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// This call is used to handle the next connection to a client. Right now, it performs
    /// the following steps:
    ///
    /// 1. It calls the [std::net::TcpListener::accept] method internally using the blocking API
    ///    until a client connects.
    /// 2. It reads all the telecommands from the client and parses all received data using the
    ///    user specified [TcpTcParser].
    /// 3. After reading and parsing all telecommands, it sends back all telemetry using the
    ///    user specified [TcpTmSender].
    ///
    /// The server will delay for a user-specified period if the client connects to the server
    /// for prolonged periods and there is no traffic for the server. This is the case if the
    /// client does not send any telecommands and no telemetry needs to be sent back to the client.
    pub fn handle_next_connection(
        &mut self,
    ) -> Result<ConnectionResult, TcpTmtcError<TmError, TcError>> {
        let mut connection_result = ConnectionResult::default();
        let mut current_write_idx;
        let mut next_write_idx = 0;
        let (mut stream, addr) = self.listener.accept()?;
        stream.set_nonblocking(true)?;
        connection_result.addr = Some(addr);
        current_write_idx = next_write_idx;
        loop {
            let read_result = stream.read(&mut self.tc_buffer[current_write_idx..]);
            match read_result {
                Ok(0) => {
                    // Connection closed by client. If any TC was read, parse for complete packets.
                    // After that, break the outer loop.
                    if current_write_idx > 0 {
                        self.tc_handler.handle_tc_parsing(
                            &mut self.tc_buffer,
                            &mut self.tc_receiver,
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
                    if current_write_idx == self.tc_buffer.capacity() {
                        self.tc_handler.handle_tc_parsing(
                            &mut self.tc_buffer,
                            &mut self.tc_receiver,
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
                            &mut self.tc_buffer,
                            &mut self.tc_receiver,
                            &mut connection_result,
                            current_write_idx,
                            &mut next_write_idx,
                        )?;
                        current_write_idx = next_write_idx;

                        if !self.tm_handler.handle_tm_sending(
                            &mut self.tm_buffer,
                            &mut self.tm_source,
                            &mut connection_result,
                            &mut stream,
                        )? {
                            // No TC read, no TM was sent, but the client has not disconnected.
                            // Perform an inner delay to avoid burning CPU time.
                            thread::sleep(self.inner_loop_delay);
                        }
                    }
                    _ => {
                        return Err(TcpTmtcError::Io(e));
                    }
                },
            }
        }
        self.tm_handler.handle_tm_sending(
            &mut self.tm_buffer,
            &mut self.tm_source,
            &mut connection_result,
            &mut stream,
        )?;
        Ok(connection_result)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use alloc::{collections::VecDeque, sync::Arc, vec::Vec};

    use crate::tmtc::{ReceivesTcCore, TmPacketSourceCore};

    #[derive(Default, Clone)]
    pub(crate) struct SyncTcCacher {
        pub(crate) tc_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    }
    impl ReceivesTcCore for SyncTcCacher {
        type Error = ();

        fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
            let mut tc_queue = self.tc_queue.lock().expect("tc forwarder failed");
            tc_queue.push_back(tc_raw.to_vec());
            Ok(())
        }
    }

    #[derive(Default, Clone)]
    pub(crate) struct SyncTmSource {
        tm_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    }

    impl SyncTmSource {
        pub(crate) fn add_tm(&mut self, tm: &[u8]) {
            let mut tm_queue = self.tm_queue.lock().expect("locking tm queue failec");
            tm_queue.push_back(tm.to_vec());
        }
    }

    impl TmPacketSourceCore for SyncTmSource {
        type Error = ();

        fn retrieve_packet(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
            let mut tm_queue = self.tm_queue.lock().expect("locking tm queue failed");
            if !tm_queue.is_empty() {
                let next_vec = tm_queue.front().unwrap();
                if buffer.len() < next_vec.len() {
                    panic!(
                        "provided buffer too small, must be at least {} bytes",
                        next_vec.len()
                    );
                }
                let next_vec = tm_queue.pop_front().unwrap();
                buffer[0..next_vec.len()].copy_from_slice(&next_vec);
                return Ok(next_vec.len());
            }
            Ok(0)
        }
    }
}
