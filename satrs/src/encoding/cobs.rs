use crate::tmtc::ReceivesTcCore;
use cobs::{decode_in_place, encode, max_encoding_length};

/// This function encodes the given packet with COBS and also wraps the encoded packet with
/// the sentinel value 0. It can be used repeatedly on the same encoded buffer by expecting
/// and incrementing the mutable reference of the current packet index. This is also used
/// to retrieve the total encoded size.
///
/// This function will return [false] if the given encoding buffer is not large enough to hold
/// the encoded buffer and the two sentinel bytes and [true] if the encoding was successfull.
///
/// ## Example
///
/// ```
/// use cobs::decode_in_place_report;
/// use satrs::encoding::{encode_packet_with_cobs};
//
/// const SIMPLE_PACKET: [u8; 5] = [1, 2, 3, 4, 5];
/// const INVERTED_PACKET: [u8; 5] = [5, 4, 3, 2, 1];
///
/// let mut encoding_buf: [u8; 32] = [0; 32];
/// let mut current_idx = 0;
/// assert!(encode_packet_with_cobs(&SIMPLE_PACKET, &mut encoding_buf, &mut current_idx));
/// assert!(encode_packet_with_cobs(&INVERTED_PACKET, &mut encoding_buf, &mut current_idx));
/// assert_eq!(encoding_buf[0], 0);
/// let dec_report = decode_in_place_report(&mut encoding_buf[1..]).expect("decoding failed");
/// assert_eq!(encoding_buf[1 + dec_report.src_used], 0);
/// assert_eq!(dec_report.dst_used, 5);
/// assert_eq!(current_idx, 16);
/// ```
pub fn encode_packet_with_cobs(
    packet: &[u8],
    encoded_buf: &mut [u8],
    current_idx: &mut usize,
) -> bool {
    let max_encoding_len = max_encoding_length(packet.len());
    if *current_idx + max_encoding_len + 2 > encoded_buf.len() {
        return false;
    }
    encoded_buf[*current_idx] = 0;
    *current_idx += 1;
    *current_idx += encode(packet, &mut encoded_buf[*current_idx..]);
    encoded_buf[*current_idx] = 0;
    *current_idx += 1;
    true
}

/// This function parses a given buffer for COBS encoded packets. The packet structure is
/// expected to be like this, assuming a sentinel value of 0 as the packet delimiter:
///
/// 0 | ... Encoded Packet Data ... | 0 | 0 | ... Encoded Packet Data ... | 0
///
/// This function is also able to deal with broken tail packets at the end. If broken tail
/// packets are detected, they are moved to the front of the buffer, and the write index for
/// future write operations will be written to the `next_write_idx` argument.
///
/// The parser will write all packets which were decoded successfully to the given `tc_receiver`.
pub fn parse_buffer_for_cobs_encoded_packets<E>(
    buf: &mut [u8],
    tc_receiver: &mut dyn ReceivesTcCore<Error = E>,
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
    // Move split frame at the end to the front of the buffer.
    if start_index_packet > 0 && start_found && packets_found > 0 {
        buf.copy_within(start_index_packet - 1.., 0);
        *next_write_idx = buf.len() - start_index_packet + 1;
    }
    Ok(packets_found)
}

#[cfg(test)]
pub(crate) mod tests {
    use cobs::encode;

    use crate::encoding::tests::{encode_simple_packet, TcCacher, INVERTED_PACKET, SIMPLE_PACKET};

    use super::parse_buffer_for_cobs_encoded_packets;

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
}
