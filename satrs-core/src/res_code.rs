use serde::{Deserialize, Serialize};
use spacepackets::ecss::{EcssEnumU16, EcssEnumeration};
use spacepackets::{ByteConversionError, SizeMissmatch};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultU16 {
    group_id: u8,
    unique_id: u8,
}

impl ResultU16 {
    pub const fn const_new(group_id: u8, unique_id: u8) -> Self {
        Self {
            group_id,
            unique_id,
        }
    }
    pub fn raw(&self) -> u16 {
        ((self.group_id as u16) << 8) | self.unique_id as u16
    }
    pub fn group_id(&self) -> u8 {
        self.group_id
    }
    pub fn unique_id(&self) -> u8 {
        self.unique_id
    }
}

impl From<ResultU16> for EcssEnumU16 {
    fn from(v: ResultU16) -> Self {
        EcssEnumU16::new(v.raw())
    }
}

impl EcssEnumeration for ResultU16 {
    fn pfc(&self) -> u8 {
        16
    }

    fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<(), ByteConversionError> {
        if buf.len() < 2 {
            return Err(ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                found: buf.len(),
                expected: 2,
            }));
        }
        buf[0] = self.group_id;
        buf[1] = self.unique_id;
        Ok(())
    }
}
