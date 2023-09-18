#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use cobs::decode_in_place;
#[cfg(feature = "alloc")]
use hashbrown::HashSet;

use crate::tmtc::ReceivesTc;

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

pub trait PacketIdLookup {
    fn validate(&self, apid: u16) -> bool;
}

#[cfg(feature = "alloc")]
impl PacketIdLookup for Vec<u16> {
    fn validate(&self, apid: u16) -> bool {
        self.contains(&apid)
    }
}

#[cfg(feature = "alloc")]
impl PacketIdLookup for HashSet<u16> {
    fn validate(&self, apid: u16) -> bool {
        self.contains(&apid)
    }
}

impl PacketIdLookup for &[u16] {
    fn validate(&self, apid: u16) -> bool {
        if self.binary_search(&apid).is_ok() {
            return true;
        }
        false
    }
}

/// This function parses a given buffer for tightly packed CCSDS space packets. It uses the
/// [PacketId] field of the CCSDS packets to detect the start of a CCSDS space packet and then
/// uses the length field of the packet to extract CCSDS packets.
///
/// This function is also able to deal with broken tail packets at the end as long a the parser
/// can read the full 6 bytes which constitue a space packet header. If broken tail packets are
/// detected, they are moved to the front of the buffer, and the write index for future write
/// operations will be written to the `next_write_idx` argument.
///
/// The parser will write all packets which were decoded successfully to the given `tc_receiver`.
pub fn parse_buffer_for_ccsds_space_packets<E>(
    buf: &mut [u8],
    packet_id_lookup: &dyn PacketIdLookup,
    tc_receiver: &mut dyn ReceivesTc<Error = E>,
    next_write_idx: &mut usize,
) -> Result<u32, E> {
    let packets_found = 0;
    let mut current_idx = 0;
    let buf_len = buf.len();
    loop {
        if current_idx + 7 >= buf.len() {
            break;
        }
        let packet_id = u16::from_be_bytes(buf[current_idx..current_idx + 2].try_into().unwrap());
        if packet_id_lookup.validate(packet_id) {
            let length_field =
                u16::from_be_bytes(buf[current_idx + 4..current_idx + 6].try_into().unwrap());
            let packet_size = length_field + 7;
            if (current_idx + packet_size as usize) < buf_len {
                tc_receiver.pass_tc(&buf[current_idx..current_idx + packet_size as usize])?;
            } else {
                // Move packet to start of buffer if applicable.
                if current_idx > 0 {
                    buf.copy_within(current_idx.., 0);
                    *next_write_idx = current_idx;
                }
            }
            current_idx += packet_size as usize;
            continue;
        }
        current_idx += 1;
    }
    Ok(packets_found)
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::{collections::VecDeque, vec::Vec};
    use cobs::encode;

    use crate::tmtc::ReceivesTcCore;

    use super::parse_buffer_for_cobs_encoded_packets;

    pub(crate) const SIMPLE_PACKET: [u8; 5] = [1, 2, 3, 4, 5];
    pub(crate) const INVERTED_PACKET: [u8; 5] = [5, 4, 3, 2, 1];

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

    pub(crate) fn encode_simple_packet(encoded_buf: &mut [u8], current_idx: &mut usize) {
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
}
