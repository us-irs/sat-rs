use spacepackets::ecss::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::time::cds::TimeProvider;
use spacepackets::time::TimeWriter;
use spacepackets::SpHeader;

#[cfg(feature = "std")]
pub use std_mod::*;

#[cfg(feature = "std")]
pub mod std_mod {
    use crate::pool::{ShareablePoolProvider, SharedPool, StoreAddr};
    use crate::pus::EcssTmtcError;
    use spacepackets::ecss::tm::{PusTm, PusTmCreator};
    use spacepackets::ecss::SerializablePusPacket;
    use std::sync::{Arc, RwLock};

    #[derive(Clone)]
    pub struct SharedTmStore {
        pool: SharedPool,
    }

    impl SharedTmStore {
        pub fn new(backing_pool: ShareablePoolProvider) -> Self {
            Self {
                pool: Arc::new(RwLock::new(backing_pool)),
            }
        }

        pub fn clone_backing_pool(&self) -> SharedPool {
            self.pool.clone()
        }

        pub fn add_pus_tm(&self, pus_tm: &PusTmCreator) -> Result<StoreAddr, EcssTmtcError> {
            let mut pg = self.pool.write().map_err(|_| EcssTmtcError::StoreLock)?;
            let (addr, buf) = pg.free_element(pus_tm.len_packed())?;
            pus_tm
                .write_to_bytes(buf)
                .expect("writing PUS TM to store failed");
            Ok(addr)
        }
    }
}

pub struct PusTmWithCdsShortHelper {
    apid: u16,
    cds_short_buf: [u8; 7],
}

impl PusTmWithCdsShortHelper {
    pub fn new(apid: u16) -> Self {
        Self {
            apid,
            cds_short_buf: [0; 7],
        }
    }

    #[cfg(feature = "std")]
    pub fn create_pus_tm_timestamp_now<'a>(
        &'a mut self,
        service: u8,
        subservice: u8,
        source_data: Option<&'a [u8]>,
        seq_count: u16,
    ) -> PusTm {
        let time_stamp = TimeProvider::from_now_with_u16_days().unwrap();
        time_stamp.write_to_bytes(&mut self.cds_short_buf).unwrap();
        self.create_pus_tm_common(service, subservice, source_data, seq_count)
    }

    pub fn create_pus_tm_with_stamper<'a>(
        &'a mut self,
        service: u8,
        subservice: u8,
        source_data: Option<&'a [u8]>,
        stamper: &TimeProvider,
        seq_count: u16,
    ) -> PusTm {
        stamper.write_to_bytes(&mut self.cds_short_buf).unwrap();
        self.create_pus_tm_common(service, subservice, source_data, seq_count)
    }

    fn create_pus_tm_common<'a>(
        &'a self,
        service: u8,
        subservice: u8,
        source_data: Option<&'a [u8]>,
        seq_count: u16,
    ) -> PusTm {
        let mut reply_header = SpHeader::tm_unseg(self.apid, seq_count, 0).unwrap();
        let tc_header = PusTmSecondaryHeader::new_simple(service, subservice, &self.cds_short_buf);
        PusTm::new(&mut reply_header, tc_header, source_data, true)
    }
}
