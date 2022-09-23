use spacepackets::time::{CdsShortTimeProvider, TimeWriter};
use spacepackets::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::SpHeader;

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
    ) -> PusTm {
        let time_stamp = CdsShortTimeProvider::from_now().unwrap();
        time_stamp.write_to_bytes(&mut self.cds_short_buf).unwrap();
        self.create_pus_tm_common(service, subservice, source_data)
    }

    pub fn create_pus_tm_with_stamp<'a>(
        &'a mut self,
        service: u8,
        subservice: u8,
        source_data: Option<&'a [u8]>,
        stamper: &CdsShortTimeProvider,
    ) -> PusTm {
        stamper.write_to_bytes(&mut self.cds_short_buf).unwrap();
        self.create_pus_tm_common(service, subservice, source_data)
    }

    fn create_pus_tm_common<'a>(
        &'a self,
        service: u8,
        subservice: u8,
        source_data: Option<&'a [u8]>,
    ) -> PusTm {
        let mut reply_header = SpHeader::tm(self.apid, 0, 0).unwrap();
        let tc_header = PusTmSecondaryHeader::new_simple(service, subservice, &self.cds_short_buf);
        PusTm::new(&mut reply_header, tc_header, source_data, true)
    }
}
