use derive_new::new;
use satrs_example::TargetIdWithApid;
use zerocopy::AsBytes;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AcsHkIds {
    TestMgmSet = 1,
}

#[derive(Debug, new, Copy, Clone)]
pub struct HkUniqueId {
    id: u32,
}

impl From<u32> for HkUniqueId {
    fn from(id: u32) -> Self {
        Self { id }
    }
}

impl HkUniqueId {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn bytes_from_target_id(&self, buf: &mut [u8], target_id: u32) -> Result<(), ()> {
        if buf.len() < 8 {
            return Err(());
        }
        buf[0..4].copy_from_slice(&self.id.to_be_bytes());
        buf[4..8].copy_from_slice(&target_id.to_be_bytes());

        Ok(())
    }

    pub fn bytes_from_target_id_with_apid(
        &self,
        buf: &mut [u8],
        target_id: TargetIdWithApid,
    ) -> Result<(), ()> {
        self.bytes_from_target_id(buf, target_id.target)
    }
}
