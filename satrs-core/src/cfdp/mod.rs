use crc::{Crc, CRC_32_CKSUM};
use spacepackets::util::UnsignedByteField;

#[cfg(feature = "std")]
pub mod dest;
pub mod user;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
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
pub enum TransactionStep {
    Idle = 0,
    TransactionStart = 1,
    ReceivingFileDataPdus = 2,
    SendingAckPdu = 3,
    TransferCompletion = 4,
    SendingFinishedPdu = 5,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum State {
    Idle = 0,
    BusyClass1Nacked = 2,
    BusyClass2Acked = 3,
}

pub const CRC_32: Crc<u32> = Crc::<u32>::new(&CRC_32_CKSUM);

#[cfg(test)]
mod tests {
    #[test]
    fn basic_test() {}
}
