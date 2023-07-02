use crate::pus::{source_buffer_large_enough, EcssTmtcError, EcssTmtcErrorWithSend};
use spacepackets::ecss::EcssEnumeration;
use spacepackets::tm::PusTm;
use spacepackets::tm::PusTmSecondaryHeader;
use spacepackets::{SpHeader, MAX_APID};

use crate::pus::EcssTmSenderCore;
#[cfg(feature = "alloc")]
pub use allocvec::EventReporter;
pub use spacepackets::ecss::event::*;

pub struct EventReporterBase {
    msg_count: u16,
    apid: u16,
    pub dest_id: u16,
}

impl EventReporterBase {
    pub fn new(apid: u16) -> Option<Self> {
        if apid > MAX_APID {
            return None;
        }
        Some(Self {
            msg_count: 0,
            dest_id: 0,
            apid,
        })
    }

    pub fn event_info<E>(
        &mut self,
        buf: &mut [u8],
        sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmtcErrorWithSend<E>> {
        self.generate_and_send_generic_tm(
            buf,
            Subservice::TmInfoReport,
            sender,
            time_stamp,
            event_id,
            aux_data,
        )
    }

    pub fn event_low_severity<E>(
        &mut self,
        buf: &mut [u8],
        sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmtcErrorWithSend<E>> {
        self.generate_and_send_generic_tm(
            buf,
            Subservice::TmLowSeverityReport,
            sender,
            time_stamp,
            event_id,
            aux_data,
        )
    }

    pub fn event_medium_severity<E>(
        &mut self,
        buf: &mut [u8],
        sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmtcErrorWithSend<E>> {
        self.generate_and_send_generic_tm(
            buf,
            Subservice::TmMediumSeverityReport,
            sender,
            time_stamp,
            event_id,
            aux_data,
        )
    }

    pub fn event_high_severity<E>(
        &mut self,
        buf: &mut [u8],
        sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmtcErrorWithSend<E>> {
        self.generate_and_send_generic_tm(
            buf,
            Subservice::TmHighSeverityReport,
            sender,
            time_stamp,
            event_id,
            aux_data,
        )
    }

    fn generate_and_send_generic_tm<E>(
        &mut self,
        buf: &mut [u8],
        subservice: Subservice,
        sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmtcErrorWithSend<E>> {
        let tm = self.generate_generic_event_tm(buf, subservice, time_stamp, event_id, aux_data)?;
        sender
            .send_tm(tm)
            .map_err(|e| EcssTmtcErrorWithSend::SendError(e))?;
        self.msg_count += 1;
        Ok(())
    }

    fn generate_generic_event_tm<'a>(
        &'a self,
        buf: &'a mut [u8],
        subservice: Subservice,
        time_stamp: &'a [u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<PusTm, EcssTmtcError> {
        let mut src_data_len = event_id.size();
        if let Some(aux_data) = aux_data {
            src_data_len += aux_data.len();
        }
        source_buffer_large_enough(buf.len(), src_data_len)?;
        let mut sp_header = SpHeader::tm_unseg(self.apid, 0, 0).unwrap();
        let sec_header = PusTmSecondaryHeader::new(
            5,
            subservice.into(),
            self.msg_count,
            self.dest_id,
            Some(time_stamp),
        );
        let mut current_idx = 0;
        event_id.write_to_be_bytes(&mut buf[0..event_id.size()])?;
        current_idx += event_id.size();
        if let Some(aux_data) = aux_data {
            buf[current_idx..current_idx + aux_data.len()].copy_from_slice(aux_data);
            current_idx += aux_data.len();
        }
        Ok(PusTm::new(
            &mut sp_header,
            sec_header,
            Some(&buf[0..current_idx]),
            true,
        ))
    }
}

#[cfg(feature = "alloc")]
mod allocvec {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    pub struct EventReporter {
        source_data_buf: Vec<u8>,
        pub reporter: EventReporterBase,
    }

    impl EventReporter {
        pub fn new(apid: u16, max_event_id_and_aux_data_size: usize) -> Option<Self> {
            let reporter = EventReporterBase::new(apid)?;
            Some(Self {
                source_data_buf: vec![0; max_event_id_and_aux_data_size],
                reporter,
            })
        }
        pub fn event_info<E>(
            &mut self,
            sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            aux_data: Option<&[u8]>,
        ) -> Result<(), EcssTmtcErrorWithSend<E>> {
            self.reporter.event_info(
                self.source_data_buf.as_mut_slice(),
                sender,
                time_stamp,
                event_id,
                aux_data,
            )
        }

        pub fn event_low_severity<E>(
            &mut self,
            sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            aux_data: Option<&[u8]>,
        ) -> Result<(), EcssTmtcErrorWithSend<E>> {
            self.reporter.event_low_severity(
                self.source_data_buf.as_mut_slice(),
                sender,
                time_stamp,
                event_id,
                aux_data,
            )
        }

        pub fn event_medium_severity<E>(
            &mut self,
            sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            aux_data: Option<&[u8]>,
        ) -> Result<(), EcssTmtcErrorWithSend<E>> {
            self.reporter.event_medium_severity(
                self.source_data_buf.as_mut_slice(),
                sender,
                time_stamp,
                event_id,
                aux_data,
            )
        }

        pub fn event_high_severity<E>(
            &mut self,
            sender: &mut (impl EcssTmSenderCore<Error = E> + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            aux_data: Option<&[u8]>,
        ) -> Result<(), EcssTmtcErrorWithSend<E>> {
            self.reporter.event_high_severity(
                self.source_data_buf.as_mut_slice(),
                sender,
                time_stamp,
                event_id,
                aux_data,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventU32, Severity};
    use crate::pus::tests::CommonTmInfo;
    use crate::SenderId;
    use spacepackets::ByteConversionError;
    use std::collections::VecDeque;
    use std::vec::Vec;

    const EXAMPLE_APID: u16 = 0xee;
    const EXAMPLE_GROUP_ID: u16 = 2;
    const EXAMPLE_EVENT_ID_0: u16 = 1;
    #[allow(dead_code)]
    const EXAMPLE_EVENT_ID_1: u16 = 2;

    #[derive(Debug, Eq, PartialEq, Clone)]
    struct TmInfo {
        pub common: CommonTmInfo,
        pub event: EventU32,
        pub aux_data: Vec<u8>,
    }

    #[derive(Default, Clone)]
    struct TestSender {
        pub service_queue: VecDeque<TmInfo>,
    }

    impl EcssTmSenderCore for TestSender {
        type Error = ();

        fn id(&self) -> SenderId {
            0
        }
        fn send_tm(&mut self, tm: PusTm) -> Result<(), Self::Error> {
            assert!(tm.source_data().is_some());
            let src_data = tm.source_data().unwrap();
            assert!(src_data.len() >= 4);
            let event = EventU32::from(u32::from_be_bytes(src_data[0..4].try_into().unwrap()));
            let mut aux_data = Vec::new();
            if src_data.len() > 4 {
                aux_data.extend_from_slice(&src_data[4..]);
            }
            self.service_queue.push_back(TmInfo {
                common: CommonTmInfo::new_from_tm(&tm),
                event,
                aux_data,
            });
            Ok(())
        }
    }

    fn severity_to_subservice(severity: Severity) -> Subservice {
        match severity {
            Severity::INFO => Subservice::TmInfoReport,
            Severity::LOW => Subservice::TmLowSeverityReport,
            Severity::MEDIUM => Subservice::TmMediumSeverityReport,
            Severity::HIGH => Subservice::TmHighSeverityReport,
        }
    }

    fn report_basic_event(
        reporter: &mut EventReporter,
        sender: &mut TestSender,
        time_stamp: &[u8],
        event: EventU32,
        severity: Severity,
        aux_data: Option<&[u8]>,
    ) {
        match severity {
            Severity::INFO => {
                reporter
                    .event_info(sender, time_stamp, event, aux_data)
                    .expect("Error reporting info event");
            }
            Severity::LOW => {
                reporter
                    .event_low_severity(sender, time_stamp, event, aux_data)
                    .expect("Error reporting low event");
            }
            Severity::MEDIUM => {
                reporter
                    .event_medium_severity(sender, time_stamp, event, aux_data)
                    .expect("Error reporting medium event");
            }
            Severity::HIGH => {
                reporter
                    .event_high_severity(sender, time_stamp, event, aux_data)
                    .expect("Error reporting high event");
            }
        }
    }

    fn basic_event_test(
        max_event_aux_data_buf: usize,
        severity: Severity,
        error_data: Option<&[u8]>,
    ) {
        let mut sender = TestSender::default();
        let reporter = EventReporter::new(EXAMPLE_APID, max_event_aux_data_buf);
        assert!(reporter.is_some());
        let mut reporter = reporter.unwrap();
        let time_stamp_empty: [u8; 7] = [0; 7];
        let mut error_copy = Vec::new();
        if let Some(err_data) = error_data {
            error_copy.extend_from_slice(err_data);
        }
        let event = EventU32::new(severity, EXAMPLE_GROUP_ID, EXAMPLE_EVENT_ID_0)
            .expect("Error creating example event");
        report_basic_event(
            &mut reporter,
            &mut sender,
            &time_stamp_empty,
            event,
            severity,
            error_data,
        );
        assert_eq!(sender.service_queue.len(), 1);
        let tm_info = sender.service_queue.pop_front().unwrap();
        assert_eq!(
            tm_info.common.subservice,
            severity_to_subservice(severity) as u8
        );
        assert_eq!(tm_info.common.dest_id, 0);
        assert_eq!(tm_info.common.time_stamp, time_stamp_empty);
        assert_eq!(tm_info.common.msg_counter, 0);
        assert_eq!(tm_info.common.apid, EXAMPLE_APID);
        assert_eq!(tm_info.event, event);
        assert_eq!(tm_info.aux_data, error_copy);
    }

    #[test]
    fn basic_info_event_generation() {
        basic_event_test(4, Severity::INFO, None);
    }

    #[test]
    fn basic_low_severity_event() {
        basic_event_test(4, Severity::LOW, None);
    }

    #[test]
    fn basic_medium_severity_event() {
        basic_event_test(4, Severity::MEDIUM, None);
    }

    #[test]
    fn basic_high_severity_event() {
        basic_event_test(4, Severity::HIGH, None);
    }

    #[test]
    fn event_with_info_string() {
        let info_string = "Test Information";
        basic_event_test(32, Severity::INFO, Some(info_string.as_bytes()));
    }

    #[test]
    fn low_severity_with_raw_err_data() {
        let raw_err_param: i32 = -1;
        let raw_err = raw_err_param.to_be_bytes();
        basic_event_test(8, Severity::LOW, Some(&raw_err))
    }

    fn check_buf_too_small(
        reporter: &mut EventReporter,
        sender: &mut TestSender,
        expected_found_len: usize,
    ) {
        let time_stamp_empty: [u8; 7] = [0; 7];
        let event = EventU32::new(Severity::INFO, EXAMPLE_GROUP_ID, EXAMPLE_EVENT_ID_0)
            .expect("Error creating example event");
        let err = reporter.event_info(sender, &time_stamp_empty, event, None);
        assert!(err.is_err());
        let err = err.unwrap_err();
        if let EcssTmErrorWithSend::EcssTmError(EcssTmtcError::ByteConversionError(
            ByteConversionError::ToSliceTooSmall(missmatch),
        )) = err
        {
            assert_eq!(missmatch.expected, 4);
            assert_eq!(missmatch.found, expected_found_len);
        } else {
            panic!("Unexpected error {:?}", err);
        }
    }

    #[test]
    fn insufficient_buffer() {
        let mut sender = TestSender::default();
        for i in 0..3 {
            let reporter = EventReporter::new(EXAMPLE_APID, i);
            assert!(reporter.is_some());
            let mut reporter = reporter.unwrap();
            check_buf_too_small(&mut reporter, &mut sender, i);
        }
    }
}
