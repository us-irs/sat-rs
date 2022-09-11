use crate::pus::{source_buffer_large_enough, EcssTmError, EcssTmSender};
use spacepackets::ecss::EcssEnumeration;
use spacepackets::tm::PusTm;
use spacepackets::tm::PusTmSecondaryHeader;
use spacepackets::{SpHeader, MAX_APID};

#[cfg(feature = "alloc")]
pub use allocvec::EventReporterWithVec;

pub struct EventReporter {
    msg_count: u16,
    apid: u16,
    pub dest_id: u16,
}

impl EventReporter {
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
        let mut src_data_len = event_id.byte_width();
        if let Some(aux_data) = aux_data {
            src_data_len += aux_data.len();
        }
        source_buffer_large_enough(buf.len(), src_data_len)?;
        let mut sp_header = SpHeader::tm(self.apid, 0, 0).unwrap();
        let sec_header = PusTmSecondaryHeader::new(5, 0, self.msg_count, self.dest_id, time_stamp);
        let mut current_idx = 0;
        event_id.write_to_bytes(&mut buf[0..event_id.byte_width()])?;
        current_idx += event_id.byte_width();
        if let Some(aux_data) = aux_data {
            buf[current_idx..current_idx + aux_data.len()].copy_from_slice(aux_data);
            current_idx += aux_data.len();
        }
        let tm = PusTm::new(&mut sp_header, sec_header, Some(&buf[0..current_idx]), true);
        sender.send_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }
}

#[cfg(feature = "alloc")]
mod allocvec {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    pub struct EventReporterWithVec {
        source_data_buf: Vec<u8>,
        pub reporter: EventReporter,
    }

    impl EventReporterWithVec {
        pub fn new(apid: u16, max_event_id_and_aux_data: usize) -> Option<Self> {
            let reporter = EventReporter::new(apid)?;
            Some(Self {
                source_data_buf: vec![0; max_event_id_and_aux_data],
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
        // pub fn event_low_severity<E>(
        //     &mut self,
        //     _sender: &mut (impl EcssTmSender<E> + ?Sized),
        //     _event_id: impl EcssEnumeration,
        //     _aux_data: Option<&[u8]>,
        // ) {
        // }
        //
        // pub fn event_medium_severity<E>(
        //     &mut self,
        //     _sender: &mut (impl EcssTmSender<E> + ?Sized),
        //     _event_id: impl EcssEnumeration,
        //     _aux_data: Option<&[u8]>,
        // ) {
        // }
        //
        // pub fn event_high_severity<E>(
        //     &mut self,
        //     _sender: &mut (impl EcssTmSender<E> + ?Sized),
        //     _event_id: impl EcssEnumeration,
        //     _aux_data: Option<&[u8]>,
        // ) {
        // }
    }
}
