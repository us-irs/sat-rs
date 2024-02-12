use derive_new::new;
use satrs::spacepackets::ByteConversionError;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AcsHkIds {
    TestMgmSet = 1,
}

#[derive(Debug, new, Copy, Clone)]
pub struct HkUniqueId {
    target_id: u32,
    set_id: u32,
}

impl HkUniqueId {
    #[allow(dead_code)]
    pub fn target_id(&self) -> u32 {
        self.target_id
    }
    #[allow(dead_code)]
    pub fn set_id(&self) -> u32 {
        self.set_id
    }

    pub fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError> {
        if buf.len() < 8 {
            return Err(ByteConversionError::ToSliceTooSmall {
                found: buf.len(),
                expected: 8,
            });
        }
        buf[0..4].copy_from_slice(&self.target_id.to_be_bytes());
        buf[4..8].copy_from_slice(&self.set_id.to_be_bytes());

        Ok(8)
    }
}
