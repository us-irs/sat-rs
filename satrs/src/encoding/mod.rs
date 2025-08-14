pub mod ccsds;
pub mod cobs;

pub use crate::encoding::ccsds::parse_buffer_for_ccsds_space_packets;
pub use crate::encoding::cobs::{encode_packet_with_cobs, parse_buffer_for_cobs_encoded_packets};

#[cfg(test)]
pub(crate) mod tests {
    use core::cell::RefCell;

    use alloc::collections::VecDeque;

    use crate::{
        ComponentId,
        tmtc::{PacketAsVec, PacketSenderRaw},
    };

    use super::cobs::encode_packet_with_cobs;

    pub(crate) const SIMPLE_PACKET: [u8; 5] = [1, 2, 3, 4, 5];
    pub(crate) const INVERTED_PACKET: [u8; 5] = [5, 4, 3, 2, 1];

    #[derive(Default)]
    pub(crate) struct TcCacher {
        pub(crate) tc_queue: RefCell<VecDeque<PacketAsVec>>,
    }

    impl PacketSenderRaw for TcCacher {
        type Error = ();

        fn send_packet(&self, sender_id: ComponentId, tc_raw: &[u8]) -> Result<(), Self::Error> {
            let mut mut_queue = self.tc_queue.borrow_mut();
            mut_queue.push_back(PacketAsVec::new(sender_id, tc_raw.to_vec()));
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
