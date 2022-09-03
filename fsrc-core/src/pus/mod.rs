use crate::pool::StoreError;
use spacepackets::time::TimestampError;
use spacepackets::ByteConversionError;

pub mod verification;

#[derive(Debug, Clone)]
pub enum SendStoredTmError<E> {
    SendError(E),
    TimeStampError(TimestampError),
    ToFromBytesError(ByteConversionError),
    StoreError(StoreError),
}
