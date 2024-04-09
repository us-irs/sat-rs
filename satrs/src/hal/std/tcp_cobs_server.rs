use alloc::sync::Arc;
use alloc::vec;
use cobs::encode;
use core::sync::atomic::AtomicBool;
use delegate::delegate;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::vec::Vec;

use crate::encoding::parse_buffer_for_cobs_encoded_packets;
use crate::tmtc::ReceivesTc;
use crate::tmtc::TmPacketSource;

use crate::hal::std::tcp_server::{
    ConnectionResult, ServerConfig, TcpTcParser, TcpTmSender, TcpTmtcError, TcpTmtcGenericServer,
};

/// Concrete [TcpTcParser] implementation for the [TcpTmtcInCobsServer].
#[derive(Default)]
pub struct CobsTcParser {}

impl<TmError, TcError: 'static> TcpTcParser<TmError, TcError> for CobsTcParser {
    fn handle_tc_parsing(
        &mut self,
        tc_buffer: &mut [u8],
        tc_receiver: &mut (impl ReceivesTc<Error = TcError> + ?Sized),
        conn_result: &mut ConnectionResult,
        current_write_idx: usize,
        next_write_idx: &mut usize,
    ) -> Result<(), TcpTmtcError<TmError, TcError>> {
        conn_result.num_received_tcs += parse_buffer_for_cobs_encoded_packets(
            &mut tc_buffer[..current_write_idx],
            tc_receiver.upcast_mut(),
            next_write_idx,
        )
        .map_err(|e| TcpTmtcError::TcError(e))?;
        Ok(())
    }
}

/// Concrete [TcpTmSender] implementation for the [TcpTmtcInCobsServer].
pub struct CobsTmSender {
    tm_encoding_buffer: Vec<u8>,
}

impl CobsTmSender {
    fn new(tm_buffer_size: usize) -> Self {
        Self {
            // The buffer should be large enough to hold the maximum expected TM size encoded with
            // COBS.
            tm_encoding_buffer: vec![0; cobs::max_encoding_length(tm_buffer_size)],
        }
    }
}

impl<TmError, TcError> TcpTmSender<TmError, TcError> for CobsTmSender {
    fn handle_tm_sending(
        &mut self,
        tm_buffer: &mut [u8],
        tm_source: &mut (impl TmPacketSource<Error = TmError> + ?Sized),
        conn_result: &mut ConnectionResult,
        stream: &mut TcpStream,
    ) -> Result<bool, TcpTmtcError<TmError, TcError>> {
        let mut tm_was_sent = false;
        loop {
            // Write TM until TM source is exhausted. For now, there is no limit for the amount
            // of TM written this way.
            let read_tm_len = tm_source
                .retrieve_packet(tm_buffer)
                .map_err(|e| TcpTmtcError::TmError(e))?;

            if read_tm_len == 0 {
                return Ok(tm_was_sent);
            }
            tm_was_sent = true;
            conn_result.num_sent_tms += 1;

            // Encode into COBS and sent to client.
            let mut current_idx = 0;
            self.tm_encoding_buffer[current_idx] = 0;
            current_idx += 1;
            current_idx += encode(
                &tm_buffer[..read_tm_len],
                &mut self.tm_encoding_buffer[current_idx..],
            );
            self.tm_encoding_buffer[current_idx] = 0;
            current_idx += 1;
            stream.write_all(&self.tm_encoding_buffer[..current_idx])?;
        }
    }
}

/// TCP TMTC server implementation for exchange of generic TMTC packets which are framed with the
/// [COBS protocol](https://en.wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing).
///
/// Telemetry will be encoded with the COBS  protocol using [cobs::encode] in addition to being
/// wrapped with the sentinel value 0 as the packet delimiter as well before being sent back to
/// the client. Please note that the server will send as much data as it can retrieve from the
/// [TmPacketSource] in its current implementation.
///
/// Using a framing protocol like COBS imposes minimal restrictions on the type of TMTC data
/// exchanged while also allowing packets with flexible size and a reliable way to reconstruct full
/// packets even from a data stream which is split up. The server wil use the
/// [parse_buffer_for_cobs_encoded_packets] function to parse for packets and pass them to a
/// generic TC receiver. The user can use [crate::encoding::encode_packet_with_cobs] to encode
/// telecommands sent to the server.
///
/// ## Example
///
/// The [TCP integration tests](https://egit.irs.uni-stuttgart.de/rust/sat-rs/src/branch/main/satrs/tests/tcp_servers.rs)
/// test also serves as the example application for this module.
pub struct TcpTmtcInCobsServer<
    TmError,
    TcError: 'static,
    TmSource: TmPacketSource<Error = TmError>,
    TcReceiver: ReceivesTc<Error = TcError>,
> {
    generic_server:
        TcpTmtcGenericServer<TmError, TcError, TmSource, TcReceiver, CobsTmSender, CobsTcParser>,
}

impl<
        TmError: 'static,
        TcError: 'static,
        TmSource: TmPacketSource<Error = TmError>,
        TcReceiver: ReceivesTc<Error = TcError>,
    > TcpTmtcInCobsServer<TmError, TcError, TmSource, TcReceiver>
{
    /// Create a new TCP TMTC server which exchanges TMTC packets encoded with
    /// [COBS protocol](https://en.wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing).
    ///
    /// ## Parameter
    ///
    /// * `cfg` - Configuration of the server.
    /// * `tm_source` - Generic TM source used by the server to pull telemetry packets which are
    ///     then sent back to the client.
    /// * `tc_receiver` - Any received telecommands which were decoded successfully will be
    ///     forwarded to this TC receiver.
    pub fn new(
        cfg: ServerConfig,
        tm_source: TmSource,
        tc_receiver: TcReceiver,
        stop_signal: Option<Arc<AtomicBool>>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            generic_server: TcpTmtcGenericServer::new(
                cfg,
                CobsTcParser::default(),
                CobsTmSender::new(cfg.tm_buffer_size),
                tm_source,
                tc_receiver,
                stop_signal,
            )?,
        })
    }

    delegate! {
        to self.generic_server {
            pub fn listener(&mut self) -> &mut TcpListener;

            /// Can be used to retrieve the local assigned address of the TCP server. This is especially
            /// useful if using the port number 0 for OS auto-assignment.
            pub fn local_addr(&self) -> std::io::Result<SocketAddr>;

            /// Delegation to the [TcpTmtcGenericServer::handle_next_connection] call.
            pub fn handle_next_connection(
                &mut self,
            ) -> Result<ConnectionResult, TcpTmtcError<TmError, TcError>>;
        }
    }
}

#[cfg(test)]
mod tests {
    use core::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };
    use std::{
        io::{Read, Write},
        net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
        thread,
        time::Instant,
    };

    use crate::{
        encoding::tests::{INVERTED_PACKET, SIMPLE_PACKET},
        hal::std::tcp_server::{
            tests::{SyncTcCacher, SyncTmSource},
            ServerConfig,
        },
    };
    use alloc::sync::Arc;
    use cobs::encode;

    use super::TcpTmtcInCobsServer;

    fn encode_simple_packet(encoded_buf: &mut [u8], current_idx: &mut usize) {
        encode_packet(&SIMPLE_PACKET, encoded_buf, current_idx)
    }

    fn encode_inverted_packet(encoded_buf: &mut [u8], current_idx: &mut usize) {
        encode_packet(&INVERTED_PACKET, encoded_buf, current_idx)
    }

    fn encode_packet(packet: &[u8], encoded_buf: &mut [u8], current_idx: &mut usize) {
        encoded_buf[*current_idx] = 0;
        *current_idx += 1;
        *current_idx += encode(packet, &mut encoded_buf[*current_idx..]);
        encoded_buf[*current_idx] = 0;
        *current_idx += 1;
    }

    fn generic_tmtc_server(
        addr: &SocketAddr,
        tc_receiver: SyncTcCacher,
        tm_source: SyncTmSource,
        stop_signal: Option<Arc<AtomicBool>>,
    ) -> TcpTmtcInCobsServer<(), (), SyncTmSource, SyncTcCacher> {
        TcpTmtcInCobsServer::new(
            ServerConfig::new(*addr, Duration::from_millis(2), 1024, 1024),
            tm_source,
            tc_receiver,
            stop_signal,
        )
        .expect("TCP server generation failed")
    }

    #[test]
    fn test_server_basic_no_tm() {
        let auto_port_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
        let tc_receiver = SyncTcCacher::default();
        let tm_source = SyncTmSource::default();
        let mut tcp_server =
            generic_tmtc_server(&auto_port_addr, tc_receiver.clone(), tm_source, None);
        let dest_addr = tcp_server
            .local_addr()
            .expect("retrieving dest addr failed");
        let conn_handled: Arc<AtomicBool> = Default::default();
        let set_if_done = conn_handled.clone();
        // Call the connection handler in separate thread, does block.
        thread::spawn(move || {
            let result = tcp_server.handle_next_connection();
            if result.is_err() {
                panic!("handling connection failed: {:?}", result.unwrap_err());
            }
            let conn_result = result.unwrap();
            assert_eq!(conn_result.num_received_tcs, 1);
            assert_eq!(conn_result.num_sent_tms, 0);
            set_if_done.store(true, Ordering::Relaxed);
        });
        // Send TC to server now.
        let mut encoded_buf: [u8; 16] = [0; 16];
        let mut current_idx = 0;
        encode_simple_packet(&mut encoded_buf, &mut current_idx);
        let mut stream = TcpStream::connect(dest_addr).expect("connecting to TCP server failed");
        stream
            .write_all(&encoded_buf[..current_idx])
            .expect("writing to TCP server failed");
        drop(stream);
        // A certain amount of time is allowed for the transaction to complete.
        for _ in 0..3 {
            if !conn_handled.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
        }
        if !conn_handled.load(Ordering::Relaxed) {
            panic!("connection was not handled properly");
        }
        // Check that the packet was received and decoded successfully.
        let mut tc_queue = tc_receiver
            .tc_queue
            .lock()
            .expect("locking tc queue failed");
        assert_eq!(tc_queue.len(), 1);
        assert_eq!(tc_queue.pop_front().unwrap(), &SIMPLE_PACKET);
        drop(tc_queue);
    }

    #[test]
    fn test_server_basic_multi_tm_multi_tc() {
        let auto_port_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
        let tc_receiver = SyncTcCacher::default();
        let mut tm_source = SyncTmSource::default();
        tm_source.add_tm(&INVERTED_PACKET);
        tm_source.add_tm(&SIMPLE_PACKET);
        let mut tcp_server = generic_tmtc_server(
            &auto_port_addr,
            tc_receiver.clone(),
            tm_source.clone(),
            None,
        );
        let dest_addr = tcp_server
            .local_addr()
            .expect("retrieving dest addr failed");
        let conn_handled: Arc<AtomicBool> = Default::default();
        let set_if_done = conn_handled.clone();
        // Call the connection handler in separate thread, does block.
        thread::spawn(move || {
            let result = tcp_server.handle_next_connection();
            if result.is_err() {
                panic!("handling connection failed: {:?}", result.unwrap_err());
            }
            let conn_result = result.unwrap();
            assert_eq!(conn_result.num_received_tcs, 2, "Not enough TCs received");
            assert_eq!(conn_result.num_sent_tms, 2, "Not enough TMs received");
            set_if_done.store(true, Ordering::Relaxed);
        });
        // Send TC to server now.
        let mut encoded_buf: [u8; 32] = [0; 32];
        let mut current_idx = 0;
        encode_simple_packet(&mut encoded_buf, &mut current_idx);
        encode_inverted_packet(&mut encoded_buf, &mut current_idx);
        let mut stream = TcpStream::connect(dest_addr).expect("connecting to TCP server failed");
        stream
            .set_read_timeout(Some(Duration::from_millis(10)))
            .expect("setting reas timeout failed");
        stream
            .write_all(&encoded_buf[..current_idx])
            .expect("writing to TCP server failed");
        // Done with writing.
        stream
            .shutdown(std::net::Shutdown::Write)
            .expect("shutting down write failed");
        let mut read_buf: [u8; 16] = [0; 16];
        let mut read_len_total = 0;
        // Timeout ensures this does not block forever.
        while read_len_total < 16 {
            let read_len = stream.read(&mut read_buf).expect("read failed");
            read_len_total += read_len;
            // Read until full expected size is available.
            if read_len == 16 {
                // Read first TM packet.
                current_idx = 0;
                assert_eq!(read_len, 16);
                assert_eq!(read_buf[0], 0);
                current_idx += 1;
                let mut dec_report = cobs::decode_in_place_report(&mut read_buf[current_idx..])
                    .expect("COBS decoding failed");
                assert_eq!(dec_report.dst_used, 5);
                // Skip first sentinel byte.
                assert_eq!(
                    &read_buf[current_idx..current_idx + INVERTED_PACKET.len()],
                    &INVERTED_PACKET
                );
                current_idx += dec_report.src_used;
                // End sentinel.
                assert_eq!(read_buf[current_idx], 0, "invalid sentinel end byte");
                current_idx += 1;

                // Read second TM packet.
                assert_eq!(read_buf[current_idx], 0);
                current_idx += 1;
                dec_report = cobs::decode_in_place_report(&mut read_buf[current_idx..])
                    .expect("COBS decoding failed");
                assert_eq!(dec_report.dst_used, 5);
                // Skip first sentinel byte.
                assert_eq!(
                    &read_buf[current_idx..current_idx + SIMPLE_PACKET.len()],
                    &SIMPLE_PACKET
                );
                current_idx += dec_report.src_used;
                // End sentinel.
                assert_eq!(read_buf[current_idx], 0);
                break;
            }
        }
        drop(stream);

        // A certain amount of time is allowed for the transaction to complete.
        for _ in 0..3 {
            if !conn_handled.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
        }
        if !conn_handled.load(Ordering::Relaxed) {
            panic!("connection was not handled properly");
        }
        // Check that the packet was received and decoded successfully.
        let mut tc_queue = tc_receiver
            .tc_queue
            .lock()
            .expect("locking tc queue failed");
        assert_eq!(tc_queue.len(), 2);
        assert_eq!(tc_queue.pop_front().unwrap(), &SIMPLE_PACKET);
        assert_eq!(tc_queue.pop_front().unwrap(), &INVERTED_PACKET);
        drop(tc_queue);
    }

    #[test]
    fn test_server_stop_signal() {
        let auto_port_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
        let tc_receiver = SyncTcCacher::default();
        let tm_source = SyncTmSource::default();
        let stop_signal = Arc::new(AtomicBool::new(false));
        let mut tcp_server = generic_tmtc_server(
            &auto_port_addr,
            tc_receiver.clone(),
            tm_source,
            Some(stop_signal.clone()),
        );
        let start = Instant::now();
        // Call the connection handler in separate thread, does block.
        thread::spawn(move || loop {
            let result = tcp_server.handle_next_connection();
            if result.is_err() {
                panic!("handling connection failed: {:?}", result.unwrap_err());
            }
            if Instant::now() - start > Duration::from_millis(50) {
                panic!("regular stop signal handling failed");
            }
        });
        stop_signal.store(true, Ordering::Relaxed);
    }
}
