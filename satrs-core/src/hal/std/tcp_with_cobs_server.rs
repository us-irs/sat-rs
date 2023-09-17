use alloc::boxed::Box;
use alloc::vec;
use cobs::decode_in_place;
use cobs::encode;
use delegate::delegate;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::vec::Vec;

use crate::tmtc::ReceivesTc;
use crate::tmtc::TmPacketSource;

use crate::hal::std::tcp_server::{
    ConnectionResult, ServerConfig, TcpTcParser, TcpTmSender, TcpTmtcError, TcpTmtcGenericServer,
};

/// Concrete [TcpTcParser] implementation for the [TcpTmtcInCobsServer].
#[derive(Default)]
pub struct CobsTcParser {}

impl<TmError, TcError> TcpTcParser<TmError, TcError> for CobsTcParser {
    fn handle_tc_parsing(
        &mut self,
        tc_buffer: &mut [u8],
        tc_receiver: &mut dyn ReceivesTc<Error = TcError>,
        conn_result: &mut ConnectionResult,
        current_write_idx: usize,
        next_write_idx: &mut usize,
    ) -> Result<(), TcpTmtcError<TmError, TcError>> {
        // Reader vec full, need to parse for packets.
        conn_result.num_received_tcs += parse_buffer_for_cobs_encoded_packets(
            &mut tc_buffer[..current_write_idx],
            tc_receiver,
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
        tm_source: &mut dyn TmPacketSource<Error = TmError>,
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
/// generic TC receiver.
pub struct TcpTmtcInCobsServer<TmError, TcError> {
    generic_server: TcpTmtcGenericServer<TmError, TcError, CobsTmSender, CobsTcParser>,
}

impl<TmError: 'static, TcError: 'static> TcpTmtcInCobsServer<TmError, TcError> {
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
        tm_source: Box<dyn TmPacketSource<Error = TmError> + Send>,
        tc_receiver: Box<dyn ReceivesTc<Error = TcError> + Send>,
    ) -> Result<Self, TcpTmtcError<TmError, TcError>> {
        Ok(Self {
            generic_server: TcpTmtcGenericServer::new(
                cfg,
                CobsTcParser::default(),
                CobsTmSender::new(cfg.tm_buffer_size),
                tm_source,
                tc_receiver,
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

/// This function parses a given buffer for COBS encoded packets. The packet structure is
/// expected to be like this, assuming a sentinel value of 0 as the packet delimiter:
///
/// 0 | ... Packet Data ... | 0 | 0 | ... Packet Data ... | 0
///
/// This function is also able to deal with broken tail packets at the end. If broken tail
/// packets are detected, they are moved to the front of the buffer, and the write index for
/// future write operations will be written to the `next_write_idx` argument.
///
/// The parser will write all packets which were decoded successfully to the given `tc_receiver`.
pub fn parse_buffer_for_cobs_encoded_packets<E>(
    buf: &mut [u8],
    tc_receiver: &mut dyn ReceivesTc<Error = E>,
    next_write_idx: &mut usize,
) -> Result<u32, E> {
    let mut start_index_packet = 0;
    let mut start_found = false;
    let mut last_byte = false;
    let mut packets_found = 0;
    for i in 0..buf.len() {
        if i == buf.len() - 1 {
            last_byte = true;
        }
        if buf[i] == 0 {
            if !start_found && !last_byte && buf[i + 1] == 0 {
                // Special case: Consecutive sentinel values or all zeroes.
                // Skip.
                continue;
            }
            if start_found {
                let decode_result = decode_in_place(&mut buf[start_index_packet..i]);
                if let Ok(packet_len) = decode_result {
                    packets_found += 1;
                    tc_receiver
                        .pass_tc(&buf[start_index_packet..start_index_packet + packet_len])?;
                }
                start_found = false;
            } else {
                start_index_packet = i + 1;
                start_found = true;
            }
        }
    }
    // Split frame at the end for a multi-packet frame. Move it to the front of the buffer.
    if start_index_packet > 0 && start_found && packets_found > 0 {
        let (first_seg, last_seg) = buf.split_at_mut(start_index_packet - 1);
        first_seg[..last_seg.len()].copy_from_slice(last_seg);
        *next_write_idx = last_seg.len();
    }
    Ok(packets_found)
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
        sync::Mutex,
        thread,
    };

    use crate::tmtc::{ReceivesTcCore, TmPacketSource};
    use alloc::{boxed::Box, collections::VecDeque, sync::Arc, vec::Vec};
    use cobs::encode;

    use super::{parse_buffer_for_cobs_encoded_packets, ServerConfig, TcpTmtcInCobsServer};

    const SIMPLE_PACKET: [u8; 5] = [1, 2, 3, 4, 5];
    const INVERTED_PACKET: [u8; 5] = [5, 4, 3, 2, 1];

    #[derive(Default, Clone)]
    struct SyncTcCacher {
        tc_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    }
    impl ReceivesTcCore for SyncTcCacher {
        type Error = ();

        fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
            let mut tc_queue = self.tc_queue.lock().expect("tc forwarder failed");
            tc_queue.push_back(tc_raw.to_vec());
            Ok(())
        }
    }

    #[derive(Default)]
    struct TcCacher {
        tc_queue: VecDeque<Vec<u8>>,
    }

    impl ReceivesTcCore for TcCacher {
        type Error = ();

        fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
            self.tc_queue.push_back(tc_raw.to_vec());
            Ok(())
        }
    }

    #[derive(Default, Clone)]
    struct SyncTmSource {
        tm_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    }

    impl SyncTmSource {
        pub(crate) fn add_tm(&mut self, tm: &[u8]) {
            let mut tm_queue = self.tm_queue.lock().expect("locking tm queue failec");
            tm_queue.push_back(tm.to_vec());
        }
    }

    impl TmPacketSource for SyncTmSource {
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

    fn encode_simple_packet(encoded_buf: &mut [u8], current_idx: &mut usize) {
        encoded_buf[*current_idx] = 0;
        *current_idx += 1;
        *current_idx += encode(&SIMPLE_PACKET, &mut encoded_buf[*current_idx..]);
        encoded_buf[*current_idx] = 0;
        *current_idx += 1;
    }

    #[test]
    fn test_parsing_simple_packet() {
        let mut test_sender = TcCacher::default();
        let mut encoded_buf: [u8; 16] = [0; 16];
        let mut current_idx = 0;
        encode_simple_packet(&mut encoded_buf, &mut current_idx);
        let mut next_read_idx = 0;
        let packets = parse_buffer_for_cobs_encoded_packets(
            &mut encoded_buf[0..current_idx],
            &mut test_sender,
            &mut next_read_idx,
        )
        .unwrap();
        assert_eq!(packets, 1);
        assert_eq!(test_sender.tc_queue.len(), 1);
        let packet = &test_sender.tc_queue[0];
        assert_eq!(packet, &SIMPLE_PACKET);
    }

    #[test]
    fn test_parsing_consecutive_packets() {
        let mut test_sender = TcCacher::default();
        let mut encoded_buf: [u8; 16] = [0; 16];
        let mut current_idx = 0;
        encode_simple_packet(&mut encoded_buf, &mut current_idx);

        // Second packet
        encoded_buf[current_idx] = 0;
        current_idx += 1;
        current_idx += encode(&INVERTED_PACKET, &mut encoded_buf[current_idx..]);
        encoded_buf[current_idx] = 0;
        current_idx += 1;
        let mut next_read_idx = 0;
        let packets = parse_buffer_for_cobs_encoded_packets(
            &mut encoded_buf[0..current_idx],
            &mut test_sender,
            &mut next_read_idx,
        )
        .unwrap();
        assert_eq!(packets, 2);
        assert_eq!(test_sender.tc_queue.len(), 2);
        let packet0 = &test_sender.tc_queue[0];
        assert_eq!(packet0, &SIMPLE_PACKET);
        let packet1 = &test_sender.tc_queue[1];
        assert_eq!(packet1, &INVERTED_PACKET);
    }

    #[test]
    fn test_split_tail_packet_only() {
        let mut test_sender = TcCacher::default();
        let mut encoded_buf: [u8; 16] = [0; 16];
        let mut current_idx = 0;
        encode_simple_packet(&mut encoded_buf, &mut current_idx);
        let mut next_read_idx = 0;
        let packets = parse_buffer_for_cobs_encoded_packets(
            // Cut off the sentinel byte at the end.
            &mut encoded_buf[0..current_idx - 1],
            &mut test_sender,
            &mut next_read_idx,
        )
        .unwrap();
        assert_eq!(packets, 0);
        assert_eq!(test_sender.tc_queue.len(), 0);
        assert_eq!(next_read_idx, 0);
    }

    fn generic_test_split_packet(cut_off: usize) {
        let mut test_sender = TcCacher::default();
        let mut encoded_buf: [u8; 16] = [0; 16];
        assert!(cut_off < INVERTED_PACKET.len() + 1);
        let mut current_idx = 0;
        encode_simple_packet(&mut encoded_buf, &mut current_idx);
        // Second packet
        encoded_buf[current_idx] = 0;
        let packet_start = current_idx;
        current_idx += 1;
        let encoded_len = encode(&INVERTED_PACKET, &mut encoded_buf[current_idx..]);
        assert_eq!(encoded_len, 6);
        current_idx += encoded_len;
        // We cut off the sentinel byte, so we expecte the write index to be the length of the
        // packet minus the sentinel byte plus the first sentinel byte.
        let next_expected_write_idx = 1 + encoded_len - cut_off + 1;
        encoded_buf[current_idx] = 0;
        current_idx += 1;
        let mut next_write_idx = 0;
        let expected_at_start = encoded_buf[packet_start..current_idx - cut_off].to_vec();
        let packets = parse_buffer_for_cobs_encoded_packets(
            // Cut off the sentinel byte at the end.
            &mut encoded_buf[0..current_idx - cut_off],
            &mut test_sender,
            &mut next_write_idx,
        )
        .unwrap();
        assert_eq!(packets, 1);
        assert_eq!(test_sender.tc_queue.len(), 1);
        assert_eq!(&test_sender.tc_queue[0], &SIMPLE_PACKET);
        assert_eq!(next_write_idx, next_expected_write_idx);
        assert_eq!(encoded_buf[..next_expected_write_idx], expected_at_start);
    }

    #[test]
    fn test_one_packet_and_split_tail_packet_0() {
        generic_test_split_packet(1);
    }

    #[test]
    fn test_one_packet_and_split_tail_packet_1() {
        generic_test_split_packet(2);
    }

    #[test]
    fn test_one_packet_and_split_tail_packet_2() {
        generic_test_split_packet(3);
    }

    #[test]
    fn test_zero_at_end() {
        let mut test_sender = TcCacher::default();
        let mut encoded_buf: [u8; 16] = [0; 16];
        let mut next_write_idx = 0;
        let mut current_idx = 0;
        encoded_buf[current_idx] = 5;
        current_idx += 1;
        encode_simple_packet(&mut encoded_buf, &mut current_idx);
        encoded_buf[current_idx] = 0;
        current_idx += 1;
        let packets = parse_buffer_for_cobs_encoded_packets(
            // Cut off the sentinel byte at the end.
            &mut encoded_buf[0..current_idx],
            &mut test_sender,
            &mut next_write_idx,
        )
        .unwrap();
        assert_eq!(packets, 1);
        assert_eq!(test_sender.tc_queue.len(), 1);
        assert_eq!(&test_sender.tc_queue[0], &SIMPLE_PACKET);
        assert_eq!(next_write_idx, 1);
        assert_eq!(encoded_buf[0], 0);
    }

    #[test]
    fn test_all_zeroes() {
        let mut test_sender = TcCacher::default();
        let mut all_zeroes: [u8; 5] = [0; 5];
        let mut next_write_idx = 0;
        let packets = parse_buffer_for_cobs_encoded_packets(
            // Cut off the sentinel byte at the end.
            &mut all_zeroes,
            &mut test_sender,
            &mut next_write_idx,
        )
        .unwrap();
        assert_eq!(packets, 0);
        assert!(test_sender.tc_queue.is_empty());
        assert_eq!(next_write_idx, 0);
    }

    fn generic_tmtc_server(
        addr: &SocketAddr,
        tc_receiver: SyncTcCacher,
        tm_source: SyncTmSource,
    ) -> TcpTmtcInCobsServer<(), ()> {
        TcpTmtcInCobsServer::new(
            ServerConfig::new(*addr, Duration::from_millis(2), 1024, 1024),
            Box::new(tm_source),
            Box::new(tc_receiver.clone()),
        )
        .expect("TCP server generation failed")
    }

    #[test]
    fn test_server_basic_no_tm() {
        let auto_port_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
        let tc_receiver = SyncTcCacher::default();
        let tm_source = SyncTmSource::default();
        let mut tcp_server =
            generic_tmtc_server(&auto_port_addr, tc_receiver.clone(), tm_source.clone());
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
    fn test_server_basic_no_tm_multi_tc() {}

    #[test]
    fn test_server_basic_with_tm() {
        let auto_port_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
        let tc_receiver = SyncTcCacher::default();
        let mut tm_source = SyncTmSource::default();
        tm_source.add_tm(&INVERTED_PACKET);
        let mut tcp_server =
            generic_tmtc_server(&auto_port_addr, tc_receiver.clone(), tm_source.clone());
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
            assert_eq!(conn_result.num_sent_tms, 1);
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
        // Done with writing.
        stream
            .shutdown(std::net::Shutdown::Write)
            .expect("shutting down write failed");
        let mut read_buf: [u8; 16] = [0; 16];
        let read_len = stream.read(&mut read_buf).expect("read failed");
        // 1 byte encoding overhead, 2 sentinel bytes.
        assert_eq!(read_len, 8);
        assert_eq!(read_buf[0], 0);
        assert_eq!(read_buf[read_len - 1], 0);
        let decoded_len =
            cobs::decode_in_place(&mut read_buf[1..read_len]).expect("COBS decoding failed");
        assert_eq!(decoded_len, 5);
        // Skip first sentinel byte.
        assert_eq!(&read_buf[1..1 + INVERTED_PACKET.len()], &INVERTED_PACKET);

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
}
