use crate::pool::StoreError;
use spacepackets::time::TimestampError;

pub mod verification;

#[derive(Debug, Copy, Clone)]
pub enum SendStoredTmError<E> {
    SendError(E),
    TimeStampError(TimestampError),
    StoreError(StoreError),
}
