use crate::pus::{source_buffer_large_enough, EcssTmError, EcssTmSender};
use spacepackets::ecss::EcssEnumeration;
use spacepackets::tm::PusTm;
use spacepackets::tm::PusTmSecondaryHeader;
use spacepackets::{SpHeader, MAX_APID};

#[cfg(feature = "alloc")]
pub use allocvec::EventReporter;

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum Subservices {
    TmInfoReport = 1,
    TmLowSeverityReport = 2,
    TmMediumSeverityReport = 3,
    TmHighSeverityReport = 4,
    TcEnableEventGeneration = 5,
    TcDisableEventGeneration = 6,
    TcReportDisabledList = 7,
    TmDisabledEventsReport = 8,
}

impl From<Subservices> for u8 {
    fn from(enumeration: Subservices) -> Self {
        enumeration as u8
    }
}

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
        sender: &mut (impl EcssTmSender<E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmError<E>> {
        self.generate_and_send_generic_tm(
            buf,
            Subservices::TmInfoReport,
            sender,
            time_stamp,
            event_id,
            aux_data,
        )
    }

    pub fn event_low_severity<E>(
        &mut self,
        buf: &mut [u8],
        sender: &mut (impl EcssTmSender<E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmError<E>> {
        self.generate_and_send_generic_tm(
            buf,
            Subservices::TmLowSeverityReport,
            sender,
            time_stamp,
            event_id,
            aux_data,
        )
    }

    pub fn event_medium_severity<E>(
        &mut self,
        buf: &mut [u8],
        sender: &mut (impl EcssTmSender<E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmError<E>> {
        self.generate_and_send_generic_tm(
            buf,
            Subservices::TmMediumSeverityReport,
            sender,
            time_stamp,
            event_id,
            aux_data,
        )
    }

    pub fn event_high_severity<E>(
        &mut self,
        buf: &mut [u8],
        sender: &mut (impl EcssTmSender<E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmError<E>> {
        self.generate_and_send_generic_tm(
            buf,
            Subservices::TmHighSeverityReport,
            sender,
            time_stamp,
            event_id,
            aux_data,
        )
    }

    fn generate_and_send_generic_tm<E>(
        &mut self,
        buf: &mut [u8],
        subservice: Subservices,
        sender: &mut (impl EcssTmSender<E> + ?Sized),
        time_stamp: &[u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<(), EcssTmError<E>> {
        let tm = self.generate_generic_event_tm(buf, subservice, time_stamp, event_id, aux_data)?;
        sender.send_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }

    fn generate_generic_event_tm<'a, E>(
        &'a self,
        buf: &'a mut [u8],
        subservice: Subservices,
        time_stamp: &'a [u8],
        event_id: impl EcssEnumeration,
        aux_data: Option<&[u8]>,
    ) -> Result<PusTm, EcssTmError<E>> {
        let mut src_data_len = event_id.byte_width();
        if let Some(aux_data) = aux_data {
            src_data_len += aux_data.len();
        }
        source_buffer_large_enough(buf.len(), src_data_len)?;
        let mut sp_header = SpHeader::tm(self.apid, 0, 0).unwrap();
        let sec_header = PusTmSecondaryHeader::new(
            5,
            subservice.into(),
            self.msg_count,
            self.dest_id,
            time_stamp,
        );
        let mut current_idx = 0;
        event_id.write_to_bytes(&mut buf[0..event_id.byte_width()])?;
        current_idx += event_id.byte_width();
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
            sender: &mut (impl EcssTmSender<E> + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            aux_data: Option<&[u8]>,
        ) -> Result<(), EcssTmError<E>> {
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
            sender: &mut (impl EcssTmSender<E> + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            aux_data: Option<&[u8]>,
        ) -> Result<(), EcssTmError<E>> {
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
            sender: &mut (impl EcssTmSender<E> + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            aux_data: Option<&[u8]>,
        ) -> Result<(), EcssTmError<E>> {
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
            sender: &mut (impl EcssTmSender<E> + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            aux_data: Option<&[u8]>,
        ) -> Result<(), EcssTmError<E>> {
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
    use crate::events::{Event, Severity};
    use crate::pus::tests::CommonTmInfo;
    use std::collections::VecDeque;

    const EXAMPLE_APID: u16 = 0xee;
    const EXAMPLE_GROUP_ID: u16 = 2;
    const EXAMPLE_EVENT_ID_0: u16 = 1;
    #[allow(dead_code)]
    const EXAMPLE_EVENT_ID_1: u16 = 2;

    #[derive(Debug, Eq, PartialEq)]
    struct TmInfo {
        pub common: CommonTmInfo,
        pub event: Event,
    }

    #[derive(Default)]
    struct TestSender {
        pub service_queue: VecDeque<TmInfo>,
    }

    impl EcssTmSender<()> for TestSender {
        fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmError<()>> {
            assert!(tm.source_data().is_some());
            let src_data = tm.source_data().unwrap();
            assert!(src_data.len() >= 4);
            let event = Event::try_from(u32::from_be_bytes(src_data[0..4].try_into().unwrap()));
            assert!(event.is_ok());
            let event = event.unwrap();
            self.service_queue.push_back(TmInfo {
                common: CommonTmInfo::new_from_tm(&tm),
                event,
            });
            Ok(())
        }
    }

    fn severity_to_subservice(severity: Severity) -> Subservices {
        match severity {
            Severity::INFO => Subservices::TmInfoReport,
            Severity::LOW => Subservices::TmLowSeverityReport,
            Severity::MEDIUM => Subservices::TmMediumSeverityReport,
            Severity::HIGH => Subservices::TmHighSeverityReport,
        }
    }

    fn report_basic_event(
        reporter: &mut EventReporter,
        sender: &mut TestSender,
        time_stamp: &[u8],
        event: Event,
        severity: Severity,
    ) {
        match severity {
            Severity::INFO => {
                reporter
                    .event_info(sender, time_stamp, event, None)
                    .expect("Error reporting info event");
            }
            Severity::LOW => {
                reporter
                    .event_low_severity(sender, time_stamp, event, None)
                    .expect("Error reporting low event");
            }
            Severity::MEDIUM => {
                reporter
                    .event_medium_severity(sender, time_stamp, event, None)
                    .expect("Error reporting medium event");
            }
            Severity::HIGH => {
                reporter
                    .event_high_severity(sender, time_stamp, event, None)
                    .expect("Error reporting high event");
            }
        }
    }

    fn basic_event_test(severity: Severity) {
        let mut sender = TestSender::default();
        let reporter = EventReporter::new(EXAMPLE_APID, 16);
        assert!(reporter.is_some());
        let mut reporter = reporter.unwrap();
        let time_stamp_empty: [u8; 7] = [0; 7];
        let event = Event::new(severity, EXAMPLE_GROUP_ID, EXAMPLE_EVENT_ID_0)
            .expect("Error creating example event");
        report_basic_event(
            &mut reporter,
            &mut sender,
            &time_stamp_empty,
            event,
            severity,
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
    }
    #[test]
    fn basic_info_event_generation() {
        basic_event_test(Severity::INFO);
    }

    #[test]
    fn basic_low_severity_event() {
        basic_event_test(Severity::LOW);
    }

    #[test]
    fn basic_medium_severity_event() {
        basic_event_test(Severity::MEDIUM);
    }

    #[test]
    fn basic_high_severity_event() {
        basic_event_test(Severity::HIGH);
    }
}
