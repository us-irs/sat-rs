use alloc::boxed::Box;
use alloc::vec;
use cobs::decode_in_place;
use cobs::encode;
use cobs::max_encoding_length;
use core::fmt::Display;
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
/// Using a framing protocol like COBS imposes minimal restrictions on the type of TMTC data
/// exchanged while also allowing packets with flexible size and a reliable way to reconstruct full
/// packets even from a data stream which is split up.
///
/// The server wil use the [parse_buffer_for_cobs_encoded_packets] function to parse for packets
/// and pass them to a generic TC receiver.
///
pub struct TcpTmtcInCobsServer<TcError, TmError> {
    base: TcpTmtcServerBase<TcError, TmError>,
    tm_encoding_buffer: Vec<u8>,
}

impl<TcError: 'static + Display, TmError: 'static + Display> TcpTmtcInCobsServer<TcError, TmError> {
    pub fn new<A: ToSocketAddrs>(
        addr: A,
        tm_buffer_size: usize,
        tm_source: Box<dyn TmPacketSource<Error = TmError>>,
        tc_buffer_size: usize,
        tc_receiver: Box<dyn ReceivesTc<Error = TcError>>,
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
                }
                current_write_idx = next_write_idx;
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
    use crate::tmtc::ReceivesTcCore;
    use alloc::vec::Vec;
    use cobs::encode;

    use super::parse_buffer_for_cobs_encoded_packets;

    const SIMPLE_PACKET: [u8; 5] = [1, 2, 3, 4, 5];

    #[derive(Default)]
    struct TestSender {
        received_tcs: Vec<Vec<u8>>,
    }

    impl ReceivesTcCore for TestSender {
        type Error = ();

        fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
            self.received_tcs.push(tc_raw.to_vec());
            Ok(())
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
        let mut test_sender = TestSender::default();
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
        assert_eq!(test_sender.received_tcs.len(), 1);
        let packet = &test_sender.received_tcs[0];
        assert_eq!(packet, &SIMPLE_PACKET);
    }

    #[test]
    fn test_parsing_consecutive_packets() {
        let mut test_sender = TestSender::default();
        let mut encoded_buf: [u8; 16] = [0; 16];
        let mut current_idx = 0;
        encode_simple_packet(&mut encoded_buf, &mut current_idx);

        let inverted_packet: [u8; 5] = [5, 4, 3, 2, 1];
        // Second packet
        encoded_buf[current_idx] = 0;
        current_idx += 1;
        current_idx += encode(&inverted_packet, &mut encoded_buf[current_idx..]);
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
        assert_eq!(test_sender.received_tcs.len(), 2);
        let packet0 = &test_sender.received_tcs[0];
        assert_eq!(packet0, &SIMPLE_PACKET);
        let packet1 = &test_sender.received_tcs[1];
        assert_eq!(packet1, &inverted_packet);
    }

    #[test]
    fn test_split_tail_packet_only() {
        let mut test_sender = TestSender::default();
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
        assert_eq!(test_sender.received_tcs.len(), 0);
        assert_eq!(next_read_idx, 0);
    }

    fn generic_test_split_packet(cut_off: usize) {
        let mut test_sender = TestSender::default();
        let mut encoded_buf: [u8; 16] = [0; 16];
        let inverted_packet: [u8; 5] = [5, 4, 3, 2, 1];
        assert!(cut_off < inverted_packet.len() + 1);
        let mut current_idx = 0;
        encode_simple_packet(&mut encoded_buf, &mut current_idx);
        // Second packet
        encoded_buf[current_idx] = 0;
        let packet_start = current_idx;
        current_idx += 1;
        let encoded_len = encode(&inverted_packet, &mut encoded_buf[current_idx..]);
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
        assert_eq!(test_sender.received_tcs.len(), 1);
        assert_eq!(&test_sender.received_tcs[0], &SIMPLE_PACKET);
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
        let mut test_sender = TestSender::default();
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
        assert_eq!(test_sender.received_tcs.len(), 1);
        assert_eq!(&test_sender.received_tcs[0], &SIMPLE_PACKET);
        assert_eq!(next_write_idx, 1);
        assert_eq!(encoded_buf[0], 0);
    }

    #[test]
    fn test_all_zeroes() {
        let mut test_sender = TestSender::default();
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
        assert!(test_sender.received_tcs.is_empty());
        assert_eq!(next_write_idx, 0);
    }
}
