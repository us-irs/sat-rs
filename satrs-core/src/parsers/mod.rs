pub mod ccsds;
pub mod cobs;

pub use crate::parsers::ccsds::parse_buffer_for_ccsds_space_packets;
pub use crate::parsers::cobs::parse_buffer_for_cobs_encoded_packets;

#[cfg(test)]
pub(crate) mod tests {
    use cobs::encode;

    pub(crate) const SIMPLE_PACKET: [u8; 5] = [1, 2, 3, 4, 5];
    pub(crate) const INVERTED_PACKET: [u8; 5] = [5, 4, 3, 2, 1];

    pub(crate) fn encode_simple_packet(encoded_buf: &mut [u8], current_idx: &mut usize) {
        encoded_buf[*current_idx] = 0;
        *current_idx += 1;
        *current_idx += encode(&SIMPLE_PACKET, &mut encoded_buf[*current_idx..]);
        encoded_buf[*current_idx] = 0;
        *current_idx += 1;
    }
}
