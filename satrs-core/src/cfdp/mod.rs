use crc::{Crc, CRC_32_CKSUM};
use spacepackets::{
    cfdp::{
        pdu::{FileDirectiveType, PduError, PduHeader},
        ChecksumType, PduType, TransmissionMode,
    },
    util::UnsignedByteField,
};

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
pub mod dest;
pub mod user;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityType {
    Sending,
    Receiving,
}

/// Generic abstraction for a check timer which has different functionality depending on whether
/// the using entity is the sending entity or the receiving entity for the unacknowledged
/// transmission mode.
///
/// For the sending entity, this timer determines the expiry period for declaring a check limit
/// fault after sending an EOF PDU with requested closure. This allows a timeout of the transfer.
/// Also see 4.6.3.2 of the CFDP standard.
///
/// For the receiving entity, this timer determines the expiry period for incrementing a check
/// counter after an EOF PDU is received for an incomplete file transfer. This allows out-of-order
/// reception of file data PDUs and EOF PDUs. Also see 4.6.3.3 of the CFDP standard.
pub trait CheckTimerProvider {
    fn has_expired(&self) -> bool;
}

#[cfg(feature = "alloc")]
pub trait CheckTimerCreator {
    fn get_check_limit_provider(
        local_id: &UnsignedByteField,
        remote_id: &UnsignedByteField,
        entity_type: EntityType,
    ) -> Box<dyn CheckTimerProvider>;
}

#[cfg(feature = "std")]
pub struct StdCheckTimer {
    expiry_time_seconds: u64,
    start_time: std::time::Instant
}


#[cfg(feature = "std")]
impl StdCheckTimer {
    pub fn new(expiry_time_seconds: u64) -> Self {
        Self {
            expiry_time_seconds,
            start_time: std::time::Instant::now()
        }
    }
}

#[cfg(feature = "std")]
impl CheckTimerProvider for StdCheckTimer {
    fn has_expired(&self) -> bool {
        let elapsed_time = self.start_time.elapsed();
        if elapsed_time.as_secs() > self.expiry_time_seconds {
            return true;
        }
        false
    }
}

#[derive(Debug)]
pub struct RemoteEntityConfig {
    pub entity_id: UnsignedByteField,
    pub max_file_segment_len: usize,
    pub closure_requeted_by_default: bool,
    pub crc_on_transmission_by_default: bool,
    pub default_transmission_mode: TransmissionMode,
    pub default_crc_type: ChecksumType,
    pub check_limit: u32,
}

pub trait RemoteEntityConfigProvider {
    fn get_remote_config(&self, remote_id: &UnsignedByteField) -> Option<&RemoteEntityConfig>;
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TransactionId {
    source_id: UnsignedByteField,
    seq_num: UnsignedByteField,
}

impl TransactionId {
    pub fn new(source_id: UnsignedByteField, seq_num: UnsignedByteField) -> Self {
        Self { source_id, seq_num }
    }

    pub fn source_id(&self) -> &UnsignedByteField {
        &self.source_id
    }

    pub fn seq_num(&self) -> &UnsignedByteField {
        &self.seq_num
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TransactionStep {
    Idle = 0,
    TransactionStart = 1,
    ReceivingFileDataPdus = 2,
    SendingAckPdu = 3,
    TransferCompletion = 4,
    SendingFinishedPdu = 5,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum State {
    Idle = 0,
    BusyClass1Nacked = 2,
    BusyClass2Acked = 3,
}

pub const CRC_32: Crc<u32> = Crc::<u32>::new(&CRC_32_CKSUM);

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PacketTarget {
    SourceEntity,
    DestEntity,
}

/// This is a helper struct which contains base information about a particular PDU packet.
/// This is also necessary information for CFDP packet routing. For example, some packet types
/// like file data PDUs can only be used by CFDP source entities.
pub struct PacketInfo<'raw_packet> {
    pdu_type: PduType,
    pdu_directive: Option<FileDirectiveType>,
    target: PacketTarget,
    raw_packet: &'raw_packet [u8],
}

impl<'raw> PacketInfo<'raw> {
    pub fn new(raw_packet: &'raw [u8]) -> Result<Self, PduError> {
        let (pdu_header, header_len) = PduHeader::from_bytes(raw_packet)?;
        if pdu_header.pdu_type() == PduType::FileData {
            return Ok(Self {
                pdu_type: pdu_header.pdu_type(),
                pdu_directive: None,
                target: PacketTarget::DestEntity,
                raw_packet,
            });
        }
        if pdu_header.pdu_datafield_len() < 1 {
            return Err(PduError::FormatError);
        }
        // Route depending on PDU type and directive type if applicable. Retrieve directive type
        // from the raw stream for better performance (with sanity and directive code check).
        // The routing is based on section 4.5 of the CFDP standard which specifies the PDU forwarding
        // procedure.
        let directive = FileDirectiveType::try_from(raw_packet[header_len]).map_err(|_| {
            PduError::InvalidDirectiveType {
                found: raw_packet[header_len],
                expected: None,
            }
        })?;
        let packet_target = match directive {
            // Section c) of 4.5.3: These PDUs should always be targeted towards the file sender a.k.a.
            // the source handler
            FileDirectiveType::NakPdu
            | FileDirectiveType::FinishedPdu
            | FileDirectiveType::KeepAlivePdu => PacketTarget::SourceEntity,
            // Section b) of 4.5.3: These PDUs should always be targeted towards the file receiver a.k.a.
            // the destination handler
            FileDirectiveType::MetadataPdu
            | FileDirectiveType::EofPdu
            | FileDirectiveType::PromptPdu => PacketTarget::DestEntity,
            // Section a): Recipient depends of the type of PDU that is being acknowledged. We can simply
            // extract the PDU type from the raw stream. If it is an EOF PDU, this packet is passed to
            // the source handler, for a Finished PDU, it is passed to the destination handler.
            FileDirectiveType::AckPdu => {
                let acked_directive = FileDirectiveType::try_from(raw_packet[header_len + 1])
                    .map_err(|_| PduError::InvalidDirectiveType {
                        found: raw_packet[header_len],
                        expected: None,
                    })?;
                if acked_directive == FileDirectiveType::EofPdu {
                    PacketTarget::SourceEntity
                } else if acked_directive == FileDirectiveType::FinishedPdu {
                    PacketTarget::DestEntity
                } else {
                    // TODO: Maybe a better error? This might be confusing..
                    return Err(PduError::InvalidDirectiveType {
                        found: raw_packet[header_len + 1],
                        expected: None,
                    });
                }
            }
        };
        Ok(Self {
            pdu_type: pdu_header.pdu_type(),
            pdu_directive: Some(directive),
            target: packet_target,
            raw_packet,
        })
    }

    pub fn pdu_type(&self) -> PduType {
        self.pdu_type
    }

    pub fn pdu_directive(&self) -> Option<FileDirectiveType> {
        self.pdu_directive
    }

    pub fn target(&self) -> PacketTarget {
        self.target
    }

    pub fn raw_packet(&self) -> &[u8] {
        self.raw_packet
    }
}

#[cfg(test)]
mod tests {
    use spacepackets::cfdp::{
        lv::Lv,
        pdu::{
            eof::EofPdu,
            file_data::FileDataPdu,
            metadata::{MetadataGenericParams, MetadataPdu},
            CommonPduConfig, FileDirectiveType, PduHeader,
        },
        PduType,
    };

    use crate::cfdp::PacketTarget;

    use super::PacketInfo;

    fn generic_pdu_header() -> PduHeader {
        let pdu_conf = CommonPduConfig::default();
        PduHeader::new_no_file_data(pdu_conf, 0)
    }

    #[test]
    fn test_metadata_pdu_info() {
        let mut buf: [u8; 128] = [0; 128];
        let pdu_header = generic_pdu_header();
        let metadata_params = MetadataGenericParams::default();
        let src_file_name = "hello.txt";
        let dest_file_name = "hello-dest.txt";
        let src_lv = Lv::new_from_str(src_file_name).unwrap();
        let dest_lv = Lv::new_from_str(dest_file_name).unwrap();
        let metadata_pdu = MetadataPdu::new(pdu_header, metadata_params, src_lv, dest_lv, None);
        metadata_pdu
            .write_to_bytes(&mut buf)
            .expect("writing metadata PDU failed");

        let packet_info = PacketInfo::new(&buf).expect("creating packet info failed");
        assert_eq!(packet_info.pdu_type(), PduType::FileDirective);
        assert!(packet_info.pdu_directive().is_some());
        assert_eq!(
            packet_info.pdu_directive().unwrap(),
            FileDirectiveType::MetadataPdu
        );
        assert_eq!(packet_info.target(), PacketTarget::DestEntity);
    }

    #[test]
    fn test_filedata_pdu_info() {
        let mut buf: [u8; 128] = [0; 128];
        let pdu_header = generic_pdu_header();
        let file_data_pdu = FileDataPdu::new_no_seg_metadata(pdu_header, 0, &[]);
        file_data_pdu
            .write_to_bytes(&mut buf)
            .expect("writing file data PDU failed");
        let packet_info = PacketInfo::new(&buf).expect("creating packet info failed");
        assert_eq!(packet_info.pdu_type(), PduType::FileData);
        assert!(packet_info.pdu_directive().is_none());
        assert_eq!(packet_info.target(), PacketTarget::DestEntity);
    }

    #[test]
    fn test_eof_pdu_info() {
        let mut buf: [u8; 128] = [0; 128];
        let pdu_header = generic_pdu_header();
        let eof_pdu = EofPdu::new_no_error(pdu_header, 0, 0);
        eof_pdu
            .write_to_bytes(&mut buf)
            .expect("writing file data PDU failed");
        let packet_info = PacketInfo::new(&buf).expect("creating packet info failed");
        assert_eq!(packet_info.pdu_type(), PduType::FileDirective);
        assert!(packet_info.pdu_directive().is_some());
        assert_eq!(
            packet_info.pdu_directive().unwrap(),
            FileDirectiveType::EofPdu
        );
    }
}
