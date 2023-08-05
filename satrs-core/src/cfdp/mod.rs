use crc::{Crc, CRC_32_CKSUM};

pub mod dest;

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
