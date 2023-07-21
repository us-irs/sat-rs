pub mod dest;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum TransactionStep {
    Idle = 0,
    TransactionStart = 1,
    ReceivingFileDataPdus = 2,
    SendingAckPdu = 3,
    TransferCompletion = 4,
    SendingFinishedPdu = 5,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum State {
    Idle = 0,
    BusyClass1Nacked = 2,
    BusyClass2Acked = 3,
}

#[cfg(test)]
mod tests {
    #[test]
    fn basic_test() {}
}
