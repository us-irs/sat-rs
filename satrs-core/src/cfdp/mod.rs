use crc::{Crc, CRC_32_CKSUM};
use spacepackets::{
    cfdp::{
        pdu::{FileDirectiveType, PduError, PduHeader},
        PduType,
    },
    util::UnsignedByteField,
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
pub mod dest;
pub mod user;

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
    #[test]
    fn basic_test() {}
}
