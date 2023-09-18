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
