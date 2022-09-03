use crate::pool::StoreError;
use spacepackets::ecss::PusError;
use spacepackets::time::TimestampError;
use spacepackets::ByteConversionError;

#[cfg(feature = "alloc")]
pub mod verification;

#[derive(Debug, Clone)]
pub enum SendStoredTmError<E> {
    SendError(E),
    TimeStampError(TimestampError),
    ToFromBytesError(ByteConversionError),
    PusError(PusError),
    StoreError(StoreError),
}
