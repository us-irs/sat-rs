use derive_new::new;
use satrs::hk::UniqueId;
use satrs::request::UniqueApidTargetId;
use satrs::spacepackets::ByteConversionError;

#[derive(Debug, new, Copy, Clone)]
pub struct HkUniqueId {
    target_id: UniqueApidTargetId,
    set_id: UniqueId,
}

impl HkUniqueId {
    #[allow(dead_code)]
    pub fn target_id(&self) -> UniqueApidTargetId {
        self.target_id
    }
    #[allow(dead_code)]
    pub fn set_id(&self) -> UniqueId {
        self.set_id
    }

    #[allow(dead_code)]
    pub fn write_to_be_bytes(&self, buf: &mut [u8]) -> Result<usize, ByteConversionError> {
        if buf.len() < 8 {
            return Err(ByteConversionError::ToSliceTooSmall {
                found: buf.len(),
                expected: 8,
            });
        }
        buf[0..4].copy_from_slice(&self.target_id.unique_id.to_be_bytes());
        buf[4..8].copy_from_slice(&self.set_id.to_be_bytes());

        Ok(8)
    }
}
