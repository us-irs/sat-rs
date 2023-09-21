#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "alloc")]
use hashbrown::HashSet;
use spacepackets::PacketId;

use crate::tmtc::ReceivesTcCore;

pub trait PacketIdLookup {
    fn validate(&self, packet_id: u16) -> bool;
}

#[cfg(feature = "alloc")]
impl PacketIdLookup for Vec<u16> {
    fn validate(&self, packet_id: u16) -> bool {
        self.contains(&packet_id)
    }
}

#[cfg(feature = "alloc")]
impl PacketIdLookup for HashSet<u16> {
    fn validate(&self, packet_id: u16) -> bool {
        self.contains(&packet_id)
    }
}

impl PacketIdLookup for [u16] {
    fn validate(&self, packet_id: u16) -> bool {
        self.binary_search(&packet_id).is_ok()
    }
}

#[cfg(feature = "alloc")]
impl PacketIdLookup for Vec<PacketId> {
    fn validate(&self, packet_id: u16) -> bool {
        self.contains(&PacketId::from(packet_id))
    }
}
#[cfg(feature = "alloc")]
impl PacketIdLookup for HashSet<PacketId> {
    fn validate(&self, packet_id: u16) -> bool {
        self.contains(&PacketId::from(packet_id))
    }
}

impl PacketIdLookup for [PacketId] {
    fn validate(&self, packet_id: u16) -> bool {
        self.binary_search(&PacketId::from(packet_id)).is_ok()
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
/// The parser will write all packets which were decoded successfully to the given `tc_receiver`
/// and return the number of packets found. If the [ReceivesTcCore::pass_tc] calls fails, the
/// error will be returned.
pub fn parse_buffer_for_ccsds_space_packets<E>(
    buf: &mut [u8],
    packet_id_lookup: &(impl PacketIdLookup + ?Sized),
    tc_receiver: &mut impl ReceivesTcCore<Error = E>,
    next_write_idx: &mut usize,
) -> Result<u32, E> {
    let mut packets_found = 0;
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
                packets_found += 1;
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
mod tests {
    use spacepackets::{
        ecss::{tc::PusTcCreator, SerializablePusPacket},
        PacketId, SpHeader,
    };

    use crate::encoding::tests::TcCacher;

    use super::parse_buffer_for_ccsds_space_packets;

    const TEST_APID: u16 = 0x02;

    #[test]
    fn test_basic() {
        let mut sph = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
        let ping_tc = PusTcCreator::new_simple(&mut sph, 17, 1, None, true);
        let mut buffer: [u8; 32] = [0; 32];
        let packet_len = ping_tc
            .write_to_bytes(&mut buffer)
            .expect("writing packet failed");
        let valid_packet_ids = [PacketId::const_tc(true, TEST_APID)];
        let mut tc_cacher = TcCacher::default();
        let mut next_write_idx = 0;
        let parse_result = parse_buffer_for_ccsds_space_packets(
            &mut buffer,
            valid_packet_ids.as_slice(),
            &mut tc_cacher,
            &mut next_write_idx,
        );
        assert!(parse_result.is_ok());
        let parsed_packets = parse_result.unwrap();
        assert_eq!(parsed_packets, 1);
        assert_eq!(tc_cacher.tc_queue.len(), 1);
        assert_eq!(
            tc_cacher.tc_queue.pop_front().unwrap(),
            buffer[..packet_len]
        );
    }

    #[test]
    fn test_multi_pakcet() {
        let mut sph = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
        let ping_tc = PusTcCreator::new_simple(&mut sph, 17, 1, None, true);
        let action_tc = PusTcCreator::new_simple(&mut sph, 8, 0, None, true);
        let mut buffer: [u8; 32] = [0; 32];
        let packet_len_ping = ping_tc
            .write_to_bytes(&mut buffer)
            .expect("writing packet failed");
        let _packet_len_action = action_tc
            .write_to_bytes(&mut buffer[packet_len_ping..])
            .expect("writing packet failed");
        let valid_packet_ids = [PacketId::const_tc(true, TEST_APID)];
        let mut tc_cacher = TcCacher::default();
        let mut next_write_idx = 0;
        let parse_result = parse_buffer_for_ccsds_space_packets(
            &mut buffer,
            valid_packet_ids.as_slice(),
            &mut tc_cacher,
            &mut next_write_idx,
        );
        assert!(parse_result.is_ok());
        let parsed_packets = parse_result.unwrap();
        assert_eq!(parsed_packets, 1);
        assert_eq!(tc_cacher.tc_queue.len(), 2);
        assert_eq!(
            tc_cacher.tc_queue.pop_front().unwrap(),
            buffer[..packet_len_ping]
        );
        assert_eq!(
            tc_cacher.tc_queue.pop_front().unwrap(),
            buffer[packet_len_ping..]
        );
    }
}
