#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use spacepackets::ecss::{EcssEnumU16, EcssEnumeration};
use spacepackets::util::UnsignedEnum;
use spacepackets::ByteConversionError;

/// Simple [u16] based result code type which also allows to group related resultcodes.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResultU16 {
    group_id: u8,
    unique_id: u8,
}

impl ResultU16 {
    pub const fn new(group_id: u8, unique_id: u8) -> Self {
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

impl UnsignedEnum for ResultU16 {
    fn size(&self) -> usize {
        core::mem::size_of::<u16>()
    }

    fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError> {
        if buf.len() < 2 {
            return Err(ByteConversionError::ToSliceTooSmall {
                found: buf.len(),
                expected: 2,
            });
        }
        buf[0] = self.group_id;
        buf[1] = self.unique_id;
        Ok(self.size())
    }
}

impl EcssEnumeration for ResultU16 {
    fn pfc(&self) -> u8 {
        16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESULT_CODE_CONST: ResultU16 = ResultU16::new(1, 1);

    #[test]
    pub fn test_basic() {
        let result_code = ResultU16::new(1, 1);
        assert_eq!(result_code.unique_id(), 1);
        assert_eq!(result_code.group_id(), 1);
        assert_eq!(result_code, RESULT_CODE_CONST);
        assert_eq!(result_code.raw(), (1_u16 << 8) | 1);
        assert_eq!(result_code.pfc(), 16);
        assert_eq!(result_code.size(), 2);
        let mut buf: [u8; 2] = [0; 2];
        let written = result_code.write_to_be_bytes(&mut buf).unwrap();
        assert_eq!(written, 2);
        assert_eq!(buf[0], 1);
        assert_eq!(buf[1], 1);
    }
}
