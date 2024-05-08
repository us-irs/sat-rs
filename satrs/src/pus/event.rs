use crate::pus::source_buffer_large_enough;
use spacepackets::ecss::tm::PusTmCreator;
use spacepackets::ecss::tm::PusTmSecondaryHeader;
use spacepackets::ecss::EcssEnumeration;
use spacepackets::ByteConversionError;
use spacepackets::{SpHeader, MAX_APID};

#[cfg(feature = "alloc")]
pub use alloc_mod::*;

pub use spacepackets::ecss::event::*;

pub struct EventReportCreator {
    apid: u16,
    pub dest_id: u16,
}

impl EventReportCreator {
    pub fn new(apid: u16, dest_id: u16) -> Option<Self> {
        if apid > MAX_APID {
            return None;
        }
        Some(Self { dest_id, apid })
    }

    pub fn event_info<'time, 'src_data>(
        &self,
        time_stamp: &'time [u8],
        event_id: impl EcssEnumeration,
        params: Option<&'src_data [u8]>,
        src_data_buf: &'src_data mut [u8],
    ) -> Result<PusTmCreator<'time, 'src_data>, ByteConversionError> {
        self.generate_and_send_generic_tm(
            Subservice::TmInfoReport,
            time_stamp,
            event_id,
            params,
            src_data_buf,
        )
    }

    pub fn event_low_severity<'time, 'src_data>(
        &self,
        time_stamp: &'time [u8],
        event_id: impl EcssEnumeration,
        params: Option<&'src_data [u8]>,
        src_data_buf: &'src_data mut [u8],
    ) -> Result<PusTmCreator<'time, 'src_data>, ByteConversionError> {
        self.generate_and_send_generic_tm(
            Subservice::TmLowSeverityReport,
            time_stamp,
            event_id,
            params,
            src_data_buf,
        )
    }

    pub fn event_medium_severity<'time, 'src_data>(
        &self,
        time_stamp: &'time [u8],
        event_id: impl EcssEnumeration,
        params: Option<&'src_data [u8]>,
        buf: &'src_data mut [u8],
    ) -> Result<PusTmCreator<'time, 'src_data>, ByteConversionError> {
        self.generate_and_send_generic_tm(
            Subservice::TmMediumSeverityReport,
            time_stamp,
            event_id,
            params,
            buf,
        )
    }

    pub fn event_high_severity<'time, 'src_data>(
        &self,
        time_stamp: &'time [u8],
        event_id: impl EcssEnumeration,
        params: Option<&'src_data [u8]>,
        src_data_buf: &'src_data mut [u8],
    ) -> Result<PusTmCreator<'time, 'src_data>, ByteConversionError> {
        self.generate_and_send_generic_tm(
            Subservice::TmHighSeverityReport,
            time_stamp,
            event_id,
            params,
            src_data_buf,
        )
    }

    fn generate_and_send_generic_tm<'time, 'src_data>(
        &self,
        subservice: Subservice,
        time_stamp: &'time [u8],
        event_id: impl EcssEnumeration,
        params: Option<&'src_data [u8]>,
        src_data_buf: &'src_data mut [u8],
    ) -> Result<PusTmCreator<'time, 'src_data>, ByteConversionError> {
        self.generate_generic_event_tm(subservice, time_stamp, event_id, params, src_data_buf)
    }

    fn generate_generic_event_tm<'time, 'src_data>(
        &self,
        subservice: Subservice,
        time_stamp: &'time [u8],
        event_id: impl EcssEnumeration,
        params: Option<&'src_data [u8]>,
        src_data_buf: &'src_data mut [u8],
    ) -> Result<PusTmCreator<'time, 'src_data>, ByteConversionError> {
        let mut src_data_len = event_id.size();
        if let Some(aux_data) = params {
            src_data_len += aux_data.len();
        }
        source_buffer_large_enough(src_data_buf.len(), src_data_len)?;
        let sec_header =
            PusTmSecondaryHeader::new(5, subservice.into(), 0, self.dest_id, time_stamp);
        let mut current_idx = 0;
        event_id.write_to_be_bytes(&mut src_data_buf[0..event_id.size()])?;
        current_idx += event_id.size();
        if let Some(aux_data) = params {
            src_data_buf[current_idx..current_idx + aux_data.len()].copy_from_slice(aux_data);
            current_idx += aux_data.len();
        }
        Ok(PusTmCreator::new(
            SpHeader::new_from_apid(self.apid),
            sec_header,
            &src_data_buf[0..current_idx],
            true,
        ))
    }
}

#[cfg(feature = "alloc")]
mod alloc_mod {
    use super::*;
    use crate::pus::{EcssTmSender, EcssTmtcError};
    use crate::ComponentId;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::cell::RefCell;
    use spacepackets::ecss::PusError;

    pub trait EventTmHookProvider {
        fn modify_tm(&self, tm: &mut PusTmCreator);
    }

    #[derive(Default)]
    pub struct DummyEventHook {}

    impl EventTmHookProvider for DummyEventHook {
        fn modify_tm(&self, _tm: &mut PusTmCreator) {}
    }

    pub struct EventReporter<EventTmHook: EventTmHookProvider = DummyEventHook> {
        id: ComponentId,
        // Use interior mutability pattern here. This is just an intermediate buffer to the PUS event packet
        // generation.
        source_data_buf: RefCell<Vec<u8>>,
        pub report_creator: EventReportCreator,
        pub tm_hook: EventTmHook,
    }

    impl EventReporter<DummyEventHook> {
        pub fn new(
            id: ComponentId,
            default_apid: u16,
            default_dest_id: u16,
            max_event_id_and_aux_data_size: usize,
        ) -> Option<Self> {
            let reporter = EventReportCreator::new(default_apid, default_dest_id)?;
            Some(Self {
                id,
                source_data_buf: RefCell::new(vec![0; max_event_id_and_aux_data_size]),
                report_creator: reporter,
                tm_hook: DummyEventHook::default(),
            })
        }
    }
    impl<EventTmHook: EventTmHookProvider> EventReporter<EventTmHook> {
        pub fn new_with_hook(
            id: ComponentId,
            default_apid: u16,
            default_dest_id: u16,
            max_event_id_and_aux_data_size: usize,
            tm_hook: EventTmHook,
        ) -> Option<Self> {
            let reporter = EventReportCreator::new(default_apid, default_dest_id)?;
            Some(Self {
                id,
                source_data_buf: RefCell::new(vec![0; max_event_id_and_aux_data_size]),
                report_creator: reporter,
                tm_hook,
            })
        }

        pub fn event_info(
            &self,
            sender: &(impl EcssTmSender + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            params: Option<&[u8]>,
        ) -> Result<(), EcssTmtcError> {
            let mut mut_buf = self.source_data_buf.borrow_mut();
            let mut tm_creator = self
                .report_creator
                .event_info(time_stamp, event_id, params, mut_buf.as_mut_slice())
                .map_err(PusError::ByteConversion)?;
            self.tm_hook.modify_tm(&mut tm_creator);
            sender.send_tm(self.id, tm_creator.into())?;
            Ok(())
        }

        pub fn event_low_severity(
            &self,
            sender: &(impl EcssTmSender + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            params: Option<&[u8]>,
        ) -> Result<(), EcssTmtcError> {
            let mut mut_buf = self.source_data_buf.borrow_mut();
            let mut tm_creator = self
                .report_creator
                .event_low_severity(time_stamp, event_id, params, mut_buf.as_mut_slice())
                .map_err(PusError::ByteConversion)?;
            self.tm_hook.modify_tm(&mut tm_creator);
            sender.send_tm(self.id, tm_creator.into())?;
            Ok(())
        }

        pub fn event_medium_severity(
            &self,
            sender: &(impl EcssTmSender + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            params: Option<&[u8]>,
        ) -> Result<(), EcssTmtcError> {
            let mut mut_buf = self.source_data_buf.borrow_mut();
            let mut tm_creator = self
                .report_creator
                .event_medium_severity(time_stamp, event_id, params, mut_buf.as_mut_slice())
                .map_err(PusError::ByteConversion)?;
            self.tm_hook.modify_tm(&mut tm_creator);
            sender.send_tm(self.id, tm_creator.into())?;
            Ok(())
        }

        pub fn event_high_severity(
            &self,
            sender: &(impl EcssTmSender + ?Sized),
            time_stamp: &[u8],
            event_id: impl EcssEnumeration,
            params: Option<&[u8]>,
        ) -> Result<(), EcssTmtcError> {
            let mut mut_buf = self.source_data_buf.borrow_mut();
            let mut tm_creator = self
                .report_creator
                .event_high_severity(time_stamp, event_id, params, mut_buf.as_mut_slice())
                .map_err(PusError::ByteConversion)?;
            self.tm_hook.modify_tm(&mut tm_creator);
            sender.send_tm(self.id, tm_creator.into())?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventU32, Severity};
    use crate::pus::test_util::TEST_COMPONENT_ID_0;
    use crate::pus::tests::CommonTmInfo;
    use crate::pus::{ChannelWithId, EcssTmSender, EcssTmtcError, PusTmVariant};
    use crate::ComponentId;
    use spacepackets::ecss::PusError;
    use spacepackets::ByteConversionError;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::vec::Vec;

    const EXAMPLE_APID: u16 = 0xee;
    const EXAMPLE_GROUP_ID: u16 = 2;
    const EXAMPLE_EVENT_ID_0: u16 = 1;
    #[allow(dead_code)]
    const EXAMPLE_EVENT_ID_1: u16 = 2;

    #[derive(Debug, Eq, PartialEq, Clone)]
    struct TmInfo {
        pub sender_id: ComponentId,
        pub common: CommonTmInfo,
        pub event: EventU32,
        pub aux_data: Vec<u8>,
    }

    #[derive(Default, Clone)]
    struct TestSender {
        pub service_queue: RefCell<VecDeque<TmInfo>>,
    }

    impl ChannelWithId for TestSender {
        fn id(&self) -> ComponentId {
            0
        }
    }

    impl EcssTmSender for TestSender {
        fn send_tm(&self, sender_id: ComponentId, tm: PusTmVariant) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmVariant::InStore(_) => {
                    panic!("TestSender: unexpected call with address");
                }
                PusTmVariant::Direct(tm) => {
                    assert!(!tm.source_data().is_empty());
                    let src_data = tm.source_data();
                    assert!(src_data.len() >= 4);
                    let event =
                        EventU32::from(u32::from_be_bytes(src_data[0..4].try_into().unwrap()));
                    let mut aux_data = Vec::new();
                    if src_data.len() > 4 {
                        aux_data.extend_from_slice(&src_data[4..]);
                    }
                    self.service_queue.borrow_mut().push_back(TmInfo {
                        sender_id,
                        common: CommonTmInfo::new_from_tm(&tm),
                        event,
                        aux_data,
                    });
                    Ok(())
                }
            }
        }
    }

    fn severity_to_subservice(severity: Severity) -> Subservice {
        match severity {
            Severity::Info => Subservice::TmInfoReport,
            Severity::Low => Subservice::TmLowSeverityReport,
            Severity::Medium => Subservice::TmMediumSeverityReport,
            Severity::High => Subservice::TmHighSeverityReport,
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
            Severity::Info => {
                reporter
                    .event_info(sender, time_stamp, event, aux_data)
                    .expect("Error reporting info event");
            }
            Severity::Low => {
                reporter
                    .event_low_severity(sender, time_stamp, event, aux_data)
                    .expect("Error reporting low event");
            }
            Severity::Medium => {
                reporter
                    .event_medium_severity(sender, time_stamp, event, aux_data)
                    .expect("Error reporting medium event");
            }
            Severity::High => {
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
        let reporter = EventReporter::new(
            TEST_COMPONENT_ID_0.id(),
            EXAMPLE_APID,
            0,
            max_event_aux_data_buf,
        );
        assert!(reporter.is_some());
        let mut reporter = reporter.unwrap();
        let time_stamp_empty: [u8; 7] = [0; 7];
        let mut error_copy = Vec::new();
        if let Some(err_data) = error_data {
            error_copy.extend_from_slice(err_data);
        }
        let event = EventU32::new_checked(severity, EXAMPLE_GROUP_ID, EXAMPLE_EVENT_ID_0)
            .expect("Error creating example event");
        report_basic_event(
            &mut reporter,
            &mut sender,
            &time_stamp_empty,
            event,
            severity,
            error_data,
        );
        let mut service_queue = sender.service_queue.borrow_mut();
        assert_eq!(service_queue.len(), 1);
        let tm_info = service_queue.pop_front().unwrap();
        assert_eq!(
            tm_info.common.subservice,
            severity_to_subservice(severity) as u8
        );
        assert_eq!(tm_info.common.dest_id, 0);
        assert_eq!(tm_info.common.timestamp, time_stamp_empty);
        assert_eq!(tm_info.common.msg_counter, 0);
        assert_eq!(tm_info.common.apid, EXAMPLE_APID);
        assert_eq!(tm_info.event, event);
        assert_eq!(tm_info.sender_id, TEST_COMPONENT_ID_0.id());
        assert_eq!(tm_info.aux_data, error_copy);
    }

    #[test]
    fn basic_info_event_generation() {
        basic_event_test(4, Severity::Info, None);
    }

    #[test]
    fn basic_low_severity_event() {
        basic_event_test(4, Severity::Low, None);
    }

    #[test]
    fn basic_medium_severity_event() {
        basic_event_test(4, Severity::Medium, None);
    }

    #[test]
    fn basic_high_severity_event() {
        basic_event_test(4, Severity::High, None);
    }

    #[test]
    fn event_with_info_string() {
        let info_string = "Test Information";
        basic_event_test(32, Severity::Info, Some(info_string.as_bytes()));
    }

    #[test]
    fn low_severity_with_raw_err_data() {
        let raw_err_param: i32 = -1;
        let raw_err = raw_err_param.to_be_bytes();
        basic_event_test(8, Severity::Low, Some(&raw_err))
    }

    fn check_buf_too_small(
        reporter: &mut EventReporter,
        sender: &mut TestSender,
        expected_found_len: usize,
    ) {
        let time_stamp_empty: [u8; 7] = [0; 7];
        let event = EventU32::new_checked(Severity::Info, EXAMPLE_GROUP_ID, EXAMPLE_EVENT_ID_0)
            .expect("Error creating example event");
        let err = reporter.event_info(sender, &time_stamp_empty, event, None);
        assert!(err.is_err());
        let err = err.unwrap_err();
        if let EcssTmtcError::Pus(PusError::ByteConversion(
            ByteConversionError::ToSliceTooSmall { found, expected },
        )) = err
        {
            assert_eq!(expected, 4);
            assert_eq!(found, expected_found_len);
        } else {
            panic!("Unexpected error {:?}", err);
        }
    }

    #[test]
    fn insufficient_buffer() {
        let mut sender = TestSender::default();
        for i in 0..3 {
            let reporter = EventReporter::new(0, EXAMPLE_APID, 0, i);
            assert!(reporter.is_some());
            let mut reporter = reporter.unwrap();
            check_buf_too_small(&mut reporter, &mut sender, i);
        }
    }
}
