use crate::pus::SendStoredTmError;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::mem::size_of;
use spacepackets::tc::PusTc;
use spacepackets::time::{CcsdsTimeProvider, TimeWriter, TimestampError};
use spacepackets::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::SpHeader;
use spacepackets::{CcsdsPacket, PacketId, PacketSequenceCtrl};
use std::marker::PhantomData;

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct RequestId {
    version_number: u8,
    packet_id: PacketId,
    psc: PacketSequenceCtrl,
}

impl RequestId {
    const SIZE_AS_BYTES: usize = size_of::<u32>();

    pub fn to_bytes(&self, buf: &mut [u8]) {
        let raw = ((self.version_number as u32) << 31)
            | ((self.packet_id.raw() as u32) << 16)
            | self.psc.raw() as u32;
        buf.copy_from_slice(raw.to_be_bytes().as_slice());
    }
}
impl RequestId {
    pub fn new(tc: &PusTc) -> Self {
        RequestId {
            version_number: tc.ccsds_version(),
            packet_id: tc.packet_id(),
            psc: tc.psc(),
        }
    }
}

pub trait VerificationSender<E> {
    fn send_verification_tm(&mut self, tm: PusTm) -> Result<(), SendStoredTmError<E>>;
}

pub struct VerificationReporter {
    pub apid: u16,
    pub dest_id: u16,
    msg_count: u16,
    time_stamper: Box<dyn TimeWriter>,
    time_stamp_buf: Vec<u8>,
    source_data_buf: Vec<u8>,
}

pub struct VerificationReporterCfg {
    pub apid: u16,
    pub dest_id: u16,
    pub time_stamper: Box<dyn TimeWriter>,
    pub step_field_width: u8,
    pub failure_code_field_width: u8,
    pub max_fail_data_len: usize,
    pub max_stamp_len: usize,
}

impl VerificationReporterCfg {
    pub fn new(time_stamper: impl TimeWriter + CcsdsTimeProvider + 'static, apid: u16) -> Self {
        let max_stamp_len = time_stamper.len_as_bytes();
        Self {
            apid,
            dest_id: 0,
            time_stamper: Box::new(time_stamper),
            step_field_width: size_of::<u8>() as u8,
            failure_code_field_width: size_of::<u16>() as u8,
            max_fail_data_len: 2 * size_of::<u32>(),
            max_stamp_len,
        }
    }
}

impl VerificationReporter {
    pub fn new(cfg: VerificationReporterCfg) -> Self {
        Self {
            apid: cfg.apid,
            dest_id: cfg.dest_id,
            msg_count: 0,
            time_stamper: cfg.time_stamper,
            time_stamp_buf: vec![0; cfg.max_stamp_len],
            source_data_buf: vec![
                0;
                RequestId::SIZE_AS_BYTES
                    + cfg.step_field_width as usize
                    + cfg.failure_code_field_width as usize
                    + cfg.max_fail_data_len
            ],
        }
    }

    pub fn add_tc(&mut self, req_id: RequestId) -> VerificationToken<StateNone> {
        VerificationToken::<StateNone>::new(req_id)
    }

    pub fn acceptance_success<E>(
        &mut self,
        token: VerificationToken<StateNone>,
        sender: &mut impl VerificationSender<E>,
    ) -> Result<VerificationToken<StateAccepted>, SendStoredTmError<E>> {
        let tm = self
            .create_pus_verif_tm(1, 1, &token.req_id)
            .map_err(|e| SendStoredTmError::TimeStampError(e))?;
        sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(VerificationToken {
            state: PhantomData,
            req_id: token.req_id,
        })
    }

    pub fn acceptance_failure<E>(
        mut self,
        token: VerificationToken<StateNone>,
        sender: &mut impl VerificationSender<E>,
        _failure_notice: &[u8],
    ) -> Result<(), SendStoredTmError<E>> {
        let tm = self
            .create_pus_verif_tm(1, 2, &token.req_id)
            .map_err(|e| SendStoredTmError::TimeStampError(e))?;
        sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn start_success(
        &mut self,
        token: VerificationToken<StateAccepted>,
    ) -> VerificationToken<StateStarted> {
        VerificationToken {
            state: PhantomData,
            req_id: token.req_id,
        }
    }

    pub fn start_failure(&mut self, _token: VerificationToken<StateAccepted>) {
        unimplemented!();
    }

    pub fn step_success(&mut self, _token: &VerificationToken<StateAccepted>) {
        unimplemented!();
    }

    pub fn step_failure(&mut self, _token: VerificationToken<StateAccepted>) {
        unimplemented!();
    }

    pub fn completion_success(&mut self, _token: VerificationToken<StateAccepted>) {
        unimplemented!();
    }
    pub fn completion_failure(&mut self, _token: VerificationToken<StateAccepted>) {
        unimplemented!();
    }

    fn create_pus_verif_tm(
        &mut self,
        service: u8,
        subservice: u8,
        req_id: &RequestId,
    ) -> Result<PusTm, TimestampError> {
        let source_data_len = size_of::<u32>();
        req_id.to_bytes(&mut self.source_data_buf[0..source_data_len]);
        let mut sp_header = SpHeader::tm(self.apid, 0, 0).unwrap();
        // I think it is okay to panic here. This error should never happen, I consider
        // this a configuration error.
        self.time_stamper
            .write_to_bytes(self.time_stamp_buf.as_mut_slice())?;
        let tm_sec_header = PusTmSecondaryHeader::new(
            service,
            subservice,
            self.msg_count,
            self.dest_id,
            &self.time_stamp_buf,
        );
        Ok(PusTm::new(
            &mut sp_header,
            tm_sec_header,
            Some(&self.source_data_buf[0..source_data_len]),
            true,
        ))
    }
}

pub struct VerificationToken<STATE> {
    state: PhantomData<STATE>,
    req_id: RequestId,
}

pub struct StateNone;
pub struct StateAccepted;
pub struct StateStarted;

impl<STATE> VerificationToken<STATE> {
    fn new(req_id: RequestId) -> VerificationToken<StateNone> {
        VerificationToken {
            state: PhantomData,
            req_id,
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    pub fn test_basic_type_state() {
        //let mut reporter = VerificationToken::new();
        //let mut accepted = reporter.acceptance_success();
        //let started = accepted.start_success();
        //started.completion_success();
    }
}
