use crate::tmtc::ReceivesTc;

pub trait ApidLookup {
    fn validate(&self, apid: u16) -> bool;
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
pub fn parse_buffer_for_ccsds_space_packets<E>(
    buf: &mut [u8],
    apid_lookup: &dyn ApidLookup,
    tc_receiver: &mut dyn ReceivesTc<Error = E>,
    next_write_idx: &mut usize,
) -> Result<u32, E> {
    let mut start_index_packet = 0;
    let mut start_found = false;
    let mut last_byte = false;
    let mut packets_found = 0;
    for i in 0..buf.len() {
        todo!();
    }
    // Split frame at the end for a multi-packet frame. Move it to the front of the buffer.
    if start_index_packet > 0 && start_found && packets_found > 0 {}
    Ok(packets_found)
}
