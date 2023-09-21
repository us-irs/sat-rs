pub mod ccsds;
pub mod cobs;

pub use crate::encoding::ccsds::parse_buffer_for_ccsds_space_packets;
pub use crate::encoding::cobs::{encode_packet_with_cobs, parse_buffer_for_cobs_encoded_packets};

#[cfg(test)]
pub(crate) mod tests {
    use alloc::{collections::VecDeque, vec::Vec};

    use crate::tmtc::ReceivesTcCore;

    use super::cobs::encode_packet_with_cobs;

    pub(crate) const SIMPLE_PACKET: [u8; 5] = [1, 2, 3, 4, 5];
    pub(crate) const INVERTED_PACKET: [u8; 5] = [5, 4, 3, 2, 1];

    #[derive(Default)]
    pub(crate) struct TcCacher {
        pub(crate) tc_queue: VecDeque<Vec<u8>>,
    }

    impl ReceivesTcCore for TcCacher {
        type Error = ();

        fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
            self.tc_queue.push_back(tc_raw.to_vec());
            Ok(())
        }
    }

    pub(crate) fn encode_simple_packet(encoded_buf: &mut [u8], current_idx: &mut usize) {
        encode_packet_with_cobs(&SIMPLE_PACKET, encoded_buf, current_idx);
    }

    #[allow(dead_code)]
    pub(crate) fn encode_inverted_packet(encoded_buf: &mut [u8], current_idx: &mut usize) {
        encode_packet_with_cobs(&INVERTED_PACKET, encoded_buf, current_idx);
    }
}
