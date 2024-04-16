use spacepackets::ecss::tm::{PusTmCreator, PusTmSecondaryHeader};
use spacepackets::time::cds::CdsTime;
use spacepackets::time::TimeWriter;
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
        source_data: &'a [u8],
        seq_count: u16,
    ) -> PusTmCreator {
        let time_stamp = CdsTime::now_with_u16_days().unwrap();
        time_stamp.write_to_bytes(&mut self.cds_short_buf).unwrap();
        self.create_pus_tm_common(service, subservice, source_data, seq_count)
    }

    pub fn create_pus_tm_with_stamper<'a>(
        &'a mut self,
        service: u8,
        subservice: u8,
        source_data: &'a [u8],
        stamper: &CdsTime,
        seq_count: u16,
    ) -> PusTmCreator {
        stamper.write_to_bytes(&mut self.cds_short_buf).unwrap();
        self.create_pus_tm_common(service, subservice, source_data, seq_count)
    }

    fn create_pus_tm_common<'a>(
        &'a self,
        service: u8,
        subservice: u8,
        source_data: &'a [u8],
        seq_count: u16,
    ) -> PusTmCreator {
        let reply_header = SpHeader::new_for_unseg_tm(self.apid, seq_count, 0);
        let tc_header = PusTmSecondaryHeader::new_simple(service, subservice, &self.cds_short_buf);
        PusTmCreator::new(reply_header, tc_header, source_data, true)
    }
}

#[cfg(test)]
mod tests {
    use spacepackets::{ecss::PusPacket, time::cds::CdsTime, CcsdsPacket};

    use super::PusTmWithCdsShortHelper;

    #[test]
    fn test_helper_with_stamper() {
        let mut pus_tm_helper = PusTmWithCdsShortHelper::new(0x123);
        let stamper = CdsTime::new_with_u16_days(0, 0);
        let tm = pus_tm_helper.create_pus_tm_with_stamper(17, 1, &[1, 2, 3, 4], &stamper, 25);
        assert_eq!(tm.service(), 17);
        assert_eq!(tm.subservice(), 1);
        assert_eq!(tm.user_data(), &[1, 2, 3, 4]);
        assert_eq!(tm.seq_count(), 25);
        assert_eq!(tm.timestamp(), [64, 0, 0, 0, 0, 0, 0])
    }

    #[test]
    fn test_helper_from_now() {
        let mut pus_tm_helper = PusTmWithCdsShortHelper::new(0x123);
        let tm = pus_tm_helper.create_pus_tm_timestamp_now(17, 1, &[1, 2, 3, 4], 25);
        assert_eq!(tm.service(), 17);
        assert_eq!(tm.subservice(), 1);
        assert_eq!(tm.user_data(), &[1, 2, 3, 4]);
        assert_eq!(tm.seq_count(), 25);
        assert_eq!(tm.timestamp().len(), 7);
    }
}
