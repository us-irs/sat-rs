//! Generic TCP TMTC servers with different TMTC format flavours.
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::AtomicBool;
use core::time::Duration;
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Token};
use socket2::{Domain, Socket, Type};
use std::io::{self, Read};
use std::net::SocketAddr;
use std::thread;

use crate::ComponentId;
use crate::tmtc::{PacketSenderRaw, PacketSource};
use thiserror::Error;

// Re-export the TMTC in COBS server.
pub use crate::hal::std::tcp_cobs_server::{CobsTcParser, CobsTmSender, TcpTmtcInCobsServer};
pub use crate::hal::std::tcp_spacepackets_server::{SpacepacketsTmSender, TcpSpacepacketsServer};

/// Configuration struct for the generic TCP TMTC server
///
/// ## Parameters
///
/// * `addr` - Address of the TCP server.
/// * `inner_loop_delay` - If a client connects for a longer period, but no TC is received or
///   no TM needs to be sent, the TCP server will delay for the specified amount of time
///   to reduce CPU load.
/// * `tm_buffer_size` - Size of the TM buffer used to read TM from the [PacketSource] and
///   encoding of that data. This buffer should at large enough to hold the maximum expected
///   TM size read from the packet source.
/// * `tc_buffer_size` - Size of the TC buffer used to read encoded telecommands sent from
///   the client. It is recommended to make this buffer larger to allow reading multiple
///   consecutive packets as well, for example by using common buffer sizes like 4096 or 8192
///   byte. The buffer should at the very least be large enough to hold the maximum expected
///   telecommand size.
/// * `reuse_addr` - Can be used to set the `SO_REUSEADDR` option on the raw socket. This is
///   especially useful if the address and port are static for the server. Set to false by
///   default.
/// * `reuse_port` - Can be used to set the `SO_REUSEPORT` option on the raw socket. This is
///   especially useful if the address and port are static for the server. Set to false by
///   default.
#[derive(Debug, Copy, Clone)]
pub struct ServerConfig {
    pub id: ComponentId,
    pub addr: SocketAddr,
    pub inner_loop_delay: Duration,
    pub tm_buffer_size: usize,
    pub tc_buffer_size: usize,
    pub reuse_addr: bool,
    pub reuse_port: bool,
}

impl ServerConfig {
    pub fn new(
        id: ComponentId,
        addr: SocketAddr,
        inner_loop_delay: Duration,
        tm_buffer_size: usize,
        tc_buffer_size: usize,
    ) -> Self {
        Self {
            id,
            addr,
            inner_loop_delay,
            tm_buffer_size,
            tc_buffer_size,
            reuse_addr: true,
            reuse_port: true,
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
#[derive(Debug, PartialEq, Eq)]
pub enum ConnectionResult {
    AcceptTimeout,
    HandledConnections(u32),
}

#[derive(Debug)]
pub struct HandledConnectionInfo {
    pub addr: SocketAddr,
    pub num_received_tcs: u32,
    pub num_sent_tms: u32,
    /// The generic TCP server can be stopped using an external signal. If this happened, this
    /// boolean will be set to true.
    pub stopped_by_signal: bool,
}

impl HandledConnectionInfo {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            num_received_tcs: 0,
            num_sent_tms: 0,
            stopped_by_signal: false,
        }
    }
}

pub trait HandledConnectionHandler {
    fn handled_connection(&mut self, info: HandledConnectionInfo);
}

/// Generic parser abstraction for an object which can parse for telecommands given a raw
/// bytestream received from a TCP socket and send them using a generic [PacketSenderRaw]
/// implementation. This allows different encoding schemes for telecommands.
pub trait TcpTcParser<TmError, SendError> {
    fn handle_tc_parsing(
        &mut self,
        tc_buffer: &mut [u8],
        sender_id: ComponentId,
        tc_sender: &(impl PacketSenderRaw<Error = SendError> + ?Sized),
        conn_result: &mut HandledConnectionInfo,
        current_write_idx: usize,
        next_write_idx: &mut usize,
    ) -> Result<(), TcpTmtcError<TmError, SendError>>;
}

/// Generic sender abstraction for an object which can pull telemetry from a given TM source
/// using a [PacketSource] and then send them back to a client using a given [TcpStream].
/// The concrete implementation can also perform any encoding steps which are necessary before
/// sending back the data to a client.
pub trait TcpTmSender<TmError, TcError> {
    fn handle_tm_sending(
        &mut self,
        tm_buffer: &mut [u8],
        tm_source: &mut (impl PacketSource<Error = TmError> + ?Sized),
        conn_result: &mut HandledConnectionInfo,
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
/// 2. Parsed telecommands will be sent using the [PacketSenderRaw] object.
/// 3. [TcpTmSender] to send telemetry pulled from a TM source back to the client.
/// 4. [PacketSource] as a generic TM source used by the [TcpTmSender].
///
/// It is possible to specify custom abstractions to build a dedicated TCP TMTC server without
/// having to re-implement common logic.
///
/// Currently, this framework offers the following concrete implementations:
///
/// 1. [TcpTmtcInCobsServer] to exchange TMTC wrapped inside the COBS framing protocol.
/// 2. [TcpSpacepacketsServer] to exchange space packets via TCP.
pub struct TcpTmtcGenericServer<
    TmSource: PacketSource<Error = TmError>,
    TcSender: PacketSenderRaw<Error = TcSendError>,
    TmSender: TcpTmSender<TmError, TcSendError>,
    TcParser: TcpTcParser<TmError, TcSendError>,
    HandledConnection: HandledConnectionHandler,
    TmError,
    TcSendError,
> {
    pub id: ComponentId,
    pub finished_handler: HandledConnection,
    pub(crate) listener: TcpListener,
    pub(crate) inner_loop_delay: Duration,
    pub(crate) tm_source: TmSource,
    pub(crate) tm_buffer: Vec<u8>,
    pub(crate) tc_sender: TcSender,
    pub(crate) tc_buffer: Vec<u8>,
    poll: Poll,
    events: Events,
    pub tc_handler: TcParser,
    pub tm_handler: TmSender,
    stop_signal: Option<Arc<AtomicBool>>,
}

impl<
    TmSource: PacketSource<Error = TmError>,
    TcSender: PacketSenderRaw<Error = TcSendError>,
    TmSender: TcpTmSender<TmError, TcSendError>,
    TcParser: TcpTcParser<TmError, TcSendError>,
    HandledConnection: HandledConnectionHandler,
    TmError: 'static,
    TcSendError: 'static,
>
    TcpTmtcGenericServer<
        TmSource,
        TcSender,
        TmSender,
        TcParser,
        HandledConnection,
        TmError,
        TcSendError,
    >
{
    /// Create a new generic TMTC server instance.
    ///
    /// ## Parameter
    ///
    /// * `cfg` - Configuration of the server.
    /// * `tc_parser` - Parser which extracts telecommands from the raw bytestream received from
    ///   the client.
    /// * `tm_sender` - Sends back telemetry to the client using the specified TM source.
    /// * `tm_source` - Generic TM source used by the server to pull telemetry packets which are
    ///   then sent back to the client.
    /// * `tc_sender` - Any received telecommand which was decoded successfully will be forwarded
    ///   using this TC sender.
    /// * `stop_signal` - Can be used to stop the server even if a connection is ongoing.
    pub fn new(
        cfg: ServerConfig,
        tc_parser: TcParser,
        tm_sender: TmSender,
        tm_source: TmSource,
        tc_receiver: TcSender,
        finished_handler: HandledConnection,
        stop_signal: Option<Arc<AtomicBool>>,
    ) -> Result<Self, std::io::Error> {
        // Create a TCP listener bound to two addresses.
        let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;

        socket.set_reuse_address(cfg.reuse_addr)?;
        #[cfg(unix)]
        socket.set_reuse_port(cfg.reuse_port)?;
        // MIO does not do this for us. We want the accept calls to be non-blocking.
        socket.set_nonblocking(true)?;
        let addr = (cfg.addr).into();
        socket.bind(&addr)?;
        socket.listen(128)?;

        // Create a poll instance.
        let poll = Poll::new()?;
        // Create storage for events.
        let events = Events::with_capacity(32);
        let listener: std::net::TcpListener = socket.into();
        let mut mio_listener = TcpListener::from_std(listener);

        // Start listening for incoming connections.
        poll.registry().register(
            &mut mio_listener,
            Token(0),
            Interest::READABLE | Interest::WRITABLE,
        )?;

        Ok(Self {
            id: cfg.id,
            tc_handler: tc_parser,
            tm_handler: tm_sender,
            poll,
            events,
            listener: mio_listener,
            inner_loop_delay: cfg.inner_loop_delay,
            tm_source,
            tm_buffer: vec![0; cfg.tm_buffer_size],
            tc_sender: tc_receiver,
            tc_buffer: vec![0; cfg.tc_buffer_size],
            stop_signal,
            finished_handler,
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

    /// This call is used to handle all connection from clients. Right now, it performs
    /// the following steps:
    ///
    /// 1. It calls the [std::net::TcpListener::accept] method until a client connects. An optional
    ///    timeout can be specified for non-blocking acceptance.
    /// 2. It reads all the telecommands from the client and parses all received data using the
    ///    user specified [TcpTcParser].
    /// 3. After reading and parsing all telecommands, it sends back all telemetry using the
    ///    user specified [TcpTmSender].
    ///
    /// The server will delay for a user-specified period if the client connects to the server
    /// for prolonged periods and there is no traffic for the server. This is the case if the
    /// client does not send any telecommands and no telemetry needs to be sent back to the client.
    pub fn handle_all_connections(
        &mut self,
        poll_timeout: Option<Duration>,
    ) -> Result<ConnectionResult, TcpTmtcError<TmError, TcSendError>> {
        let mut handled_connections = 0;
        // Poll Mio for events.
        self.poll.poll(&mut self.events, poll_timeout)?;
        let mut acceptable_connection = false;
        // Process each event.
        for event in self.events.iter() {
            if event.token() == Token(0) {
                acceptable_connection = true;
            } else {
                // Should never happen..
                panic!("unexpected TCP event token");
            }
        }
        // I'd love to do this in the loop above, but there are issues with multiple borrows.
        if acceptable_connection {
            // There might be mutliple connections available. Accept until all of them have
            // been handled.
            loop {
                match self.listener.accept() {
                    Ok((stream, addr)) => {
                        if let Err(e) = self.handle_accepted_connection(stream, addr) {
                            self.reregister_poll_interest()?;
                            return Err(e);
                        }
                        handled_connections += 1;
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
                    Err(err) => {
                        self.reregister_poll_interest()?;
                        return Err(TcpTmtcError::Io(err));
                    }
                }
            }
        }
        if handled_connections > 0 {
            return Ok(ConnectionResult::HandledConnections(handled_connections));
        }
        Ok(ConnectionResult::AcceptTimeout)
    }

    fn reregister_poll_interest(&mut self) -> io::Result<()> {
        self.poll.registry().reregister(
            &mut self.listener,
            Token(0),
            Interest::READABLE | Interest::WRITABLE,
        )
    }

    fn handle_accepted_connection(
        &mut self,
        mut stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), TcpTmtcError<TmError, TcSendError>> {
        let mut current_write_idx;
        let mut next_write_idx = 0;
        let mut connection_result = HandledConnectionInfo::new(addr);
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
                            self.id,
                            &self.tc_sender,
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
                            self.id,
                            &self.tc_sender,
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
                            self.id,
                            &self.tc_sender,
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
                            // Optional stop signal handling.
                            if self.stop_signal.is_some()
                                && self
                                    .stop_signal
                                    .as_ref()
                                    .unwrap()
                                    .load(std::sync::atomic::Ordering::Relaxed)
                            {
                                connection_result.stopped_by_signal = true;
                                self.finished_handler.handled_connection(connection_result);
                                return Ok(());
                            }
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
        self.finished_handler.handled_connection(connection_result);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use alloc::{collections::VecDeque, sync::Arc, vec::Vec};

    use crate::tmtc::PacketSource;

    use super::*;

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

    impl PacketSource for SyncTmSource {
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

    #[derive(Default)]
    pub struct ConnectionFinishedHandler {
        connection_info: VecDeque<HandledConnectionInfo>,
    }

    impl HandledConnectionHandler for ConnectionFinishedHandler {
        fn handled_connection(&mut self, info: HandledConnectionInfo) {
            self.connection_info.push_back(info);
        }
    }

    impl ConnectionFinishedHandler {
        pub fn check_last_connection(&mut self, num_tms: u32, num_tcs: u32) {
            let last_conn_result = self
                .connection_info
                .pop_back()
                .expect("no connection info available");
            assert_eq!(last_conn_result.num_received_tcs, num_tcs);
            assert_eq!(last_conn_result.num_sent_tms, num_tms);
        }

        pub fn check_no_connections_left(&self) {
            assert!(self.connection_info.is_empty());
        }
    }
}
