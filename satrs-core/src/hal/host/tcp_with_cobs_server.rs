use alloc::boxed::Box;
use alloc::vec;
use cobs::decode_in_place;
use cobs::encode;
use cobs::max_encoding_length;
use std::io::Read;
use std::io::Write;
use std::net::ToSocketAddrs;
use std::vec::Vec;

use crate::hal::host::tcp_server::TcpTmtcServerBase;
use crate::tmtc::ReceivesTc;
use crate::tmtc::TmPacketSource;

use super::tcp_server::ConnectionResult;
use super::tcp_server::TcpTmtcError;

/// TCP TMTC server implementation for exchange of generic TMTC packets which are framed with the
/// [COBS protocol](https://en.wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing).
///
/// TCP is stream oriented, so a client can read available telemetry using [std::io::Read] as well.
/// To allow flexibly specifying the telemetry sent back to clients, a generic TM abstraction
/// in form of the [TmPacketSource] trait is used. Telemetry will be encoded with the COBS
/// protocol using [cobs::encode] in addition to being wrapped with the sentinel value 0 as the
/// packet delimiter as well before being sent back to the client. Please note that the server
/// will send as much data as it can retrieve from the [TmPacketSource] in its current
/// implementation.
///
/// Using a framing protocol like COBS imposes minimal restrictions on the type of TMTC data
/// exchanged while also allowing packets with flexible size and a reliable way to reconstruct full
/// packets even from a data stream which is split up. The server wil use the
/// [parse_buffer_for_cobs_encoded_packets] function to parse for packets and pass them to a
/// generic TC receiver.
pub struct TcpTmtcInCobsServer<TcError, TmError> {
    base: TcpTmtcServerBase<TcError, TmError>,
    tm_encoding_buffer: Vec<u8>,
}

impl<TcError: 'static, TmError: 'static> TcpTmtcInCobsServer<TcError, TmError> {
    /// Create a new TMTC server which exchanges TMTC packets encoded with
    /// [COBS protocol](https://en.wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing).
    ///
    /// ## Parameter
    ///
    /// * `addr` - Address of the TCP server.
    /// * `tm_buffer_size` - Size of the TM buffer used to read TM from the [TmPacketSource] and
    ///     encoding of that data. This buffer should at large enough to hold the maximum expected
    ///     TM size in addition to the COBS encoding overhead. You can use
    ///     [cobs::max_encoding_length] to calculate this size.
    /// * `tm_source` - Generic TM source used by the server to pull telemetry packets which are
    ///     then sent back to the client.
    /// * `tc_buffer_size` - Size of the TC buffer used to read encoded telecommands sent from
    ///     the client. It is recommended to make this buffer larger to allow reading multiple
    ///     consecutive packets as well, for example by using 4096 or 8192 byte. The buffer should
    ///     at the very least be large enough to hold the maximum expected telecommand size in
    ///     addition to its COBS encoding overhead. You can use [cobs::max_encoding_length] to
    ///     calculate this size.
    /// * `tc_receiver` - Any received telecommand which was decoded successfully will be forwarded
    ///     to this TC receiver.
    pub fn new<A: ToSocketAddrs>(
        addr: A,
        tm_buffer_size: usize,
        tm_source: Box<dyn TmPacketSource<Error = TmError> + Send>,
        tc_buffer_size: usize,
        tc_receiver: Box<dyn ReceivesTc<Error = TcError> + Send>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            base: TcpTmtcServerBase::new(
                addr,
                tm_buffer_size,
                tm_source,
                tc_buffer_size,
                tc_receiver,
            )?,
            tm_encoding_buffer: vec![0; max_encoding_length(tc_buffer_size)],
        })
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
        connection_result.addr = Some(addr);
        current_write_idx = next_write_idx;
        next_write_idx = 0;
        loop {
            let read_len = stream.read(&mut self.base.tc_buffer[current_write_idx..])?;
            if read_len > 0 {
                current_write_idx += read_len;
                if current_write_idx == self.base.tc_buffer.capacity() {
                    // Reader vec full, need to parse for packets.
                    connection_result.num_received_tcs += parse_buffer_for_cobs_encoded_packets(
                        &mut self.base.tc_buffer[..current_write_idx],
                        self.base.tc_receiver.as_mut(),
                        &mut next_write_idx,
                    )
                    .map_err(|e| TcpTmtcError::TcError(e))?;
                    current_write_idx = next_write_idx;
                }
                continue;
            }
            break;
        }
        if current_write_idx > 0 {
            connection_result.num_received_tcs += parse_buffer_for_cobs_encoded_packets(
                &mut self.base.tc_buffer[..current_write_idx],
                self.base.tc_receiver.as_mut(),
                &mut next_write_idx,
            )
            .map_err(|e| TcpTmtcError::TcError(e))?;
        }
        loop {
            // Write TM until TM source is exhausted. For now, there is no limit for the amount
            // of TM written this way.
            let read_tm_len = self
                .base
                .tm_source
                .retrieve_packet(&mut self.base.tm_buffer)
                .map_err(|e| TcpTmtcError::TmError(e))?;
            if read_tm_len == 0 {
                break;
            }
            connection_result.num_sent_tms += 1;

            // Encode into COBS and sent to client.
            let mut current_idx = 0;
            self.tm_encoding_buffer[current_idx] = 0;
            current_idx += 1;
            current_idx += encode(
                &self.base.tm_buffer[..read_tm_len],
                &mut self.tm_encoding_buffer,
            );
            self.tm_encoding_buffer[current_idx] = 0;
            current_idx += 1;
            stream.write_all(&self.tm_encoding_buffer[..current_idx])?;
        }
        Ok(connection_result)
    }
}

/// This function parses a given buffer for COBS encoded packets. The packet structure is
/// expected to be like this, assuming a sentinel value of 0 as the packet delimiter.
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

    use super::{parse_buffer_for_cobs_encoded_packets, TcpTmtcInCobsServer};

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
            let tm_queue = self.tm_queue.lock().expect("locking tm queue failed");
            if !tm_queue.is_empty() {
                let next_vec = tm_queue.front().unwrap();
                if buffer.len() < next_vec.len() {
                    panic!(
                        "provided buffer too small, must be at least {} bytes",
                        next_vec.len()
                    );
                }
                buffer[0..next_vec.len()].copy_from_slice(next_vec);
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
            addr,
            1024,
            Box::new(tm_source),
            1024,
            Box::new(tc_receiver.clone()),
        )
        .expect("TCP server generation failed")
    }

    #[test]
    fn test_server_basic_no_tm() {
        let dest_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 7777);
        let tc_receiver = SyncTcCacher::default();
        let tm_source = SyncTmSource::default();
        let mut tcp_server =
            generic_tmtc_server(&dest_addr, tc_receiver.clone(), tm_source.clone());
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
        let dest_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 7777);
        let tc_receiver = SyncTcCacher::default();
        let mut tm_source = SyncTmSource::default();
        tm_source.add_tm(&INVERTED_PACKET);
        let mut tcp_server =
            generic_tmtc_server(&dest_addr, tc_receiver.clone(), tm_source.clone());
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
        let mut read_buf: [u8; 16] = [0; 16];
        let read_len = stream.read(&mut read_buf).expect("read failed");
        // 1 byte encoding overhead, 2 sentinel bytes.
        assert_eq!(read_len, 8);
        assert_eq!(read_buf[0], 0);
        assert_eq!(read_buf[read_len - 1], 0);
        let decoded_len =
            cobs::decode_in_place(&mut read_buf[1..read_len]).expect("COBS decoding failed");
        assert_eq!(decoded_len, 5);
        assert_eq!(&read_buf[..INVERTED_PACKET.len()], &INVERTED_PACKET);

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
