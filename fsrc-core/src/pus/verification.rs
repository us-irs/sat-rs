use crate::pus::SendStoredTmError;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::mem::size_of;
use spacepackets::ecss::EcssEnumeration;
use spacepackets::tc::PusTc;
use spacepackets::time::{CcsdsTimeProvider, TimeWriter};
use spacepackets::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::{ByteConversionError, SizeMissmatch, SpHeader};
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

pub struct FailParams<'a, E> {
    sender: &'a mut dyn VerificationSender<E>,
    failure_code: &'a dyn EcssEnumeration,
    failure_data: &'a [u8],
}

impl<'a, E> FailParams<'a, E> {
    pub fn new(
        sender: &'a mut impl VerificationSender<E>,
        failure_code: &'a impl EcssEnumeration,
        failure_data: &'a [u8],
    ) -> Self {
        Self {
            sender,
            failure_code,
            failure_data,
        }
    }
}

pub struct FailParamsWithStep<'a, E> {
    bp: FailParams<'a, E>,
    step: &'a dyn EcssEnumeration,
}

impl<'a, E> FailParamsWithStep<'a, E> {
    pub fn new(
        sender: &'a mut impl VerificationSender<E>,
        failure_code: &'a impl EcssEnumeration,
        failure_data: &'a [u8],
        step: &'a impl EcssEnumeration,
    ) -> Self {
        Self {
            bp: FailParams::new(sender, failure_code, failure_data),
            step,
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
        let test: Option<&dyn EcssEnumeration> = None;
        let tm = self.create_pus_verif_success_tm(1, 1, &token.req_id, test)?;
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
        params: FailParams<E>,
    ) -> Result<(), SendStoredTmError<E>> {
        let tm = self.create_pus_verif_fail_tm(
            1,
            2,
            &token.req_id,
            None::<&dyn EcssEnumeration>,
            params.failure_code,
            params.failure_data,
        )?;
        params.sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn start_success<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        sender: &mut impl VerificationSender<E>,
    ) -> Result<VerificationToken<StateStarted>, SendStoredTmError<E>> {
        let tm =
            self.create_pus_verif_success_tm(1, 3, &token.req_id, None::<&dyn EcssEnumeration>)?;
        sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(VerificationToken {
            state: PhantomData,
            req_id: token.req_id,
        })
    }

    pub fn start_failure<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        params: FailParams<E>,
    ) -> Result<(), SendStoredTmError<E>> {
        let tm = self.create_pus_verif_fail_tm(
            1,
            4,
            &token.req_id,
            None::<&dyn EcssEnumeration>,
            params.failure_code,
            params.failure_data,
        )?;
        params.sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn step_success<E>(
        &mut self,
        token: &VerificationToken<StateAccepted>,
        sender: &mut impl VerificationSender<E>,
        step: impl EcssEnumeration,
    ) -> Result<(), SendStoredTmError<E>> {
        let tm = self.create_pus_verif_success_tm(1, 5, &token.req_id, Some(&step))?;
        sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn step_failure<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        params: FailParamsWithStep<E>,
    ) -> Result<(), SendStoredTmError<E>> {
        let tm = self.create_pus_verif_fail_tm(
            1,
            6,
            &token.req_id,
            Some(params.step),
            params.bp.failure_code,
            params.bp.failure_data,
        )?;
        params.bp.sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn completion_success<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        sender: &mut impl VerificationSender<E>,
    ) -> Result<(), SendStoredTmError<E>> {
        let tm =
            self.create_pus_verif_success_tm(1, 7, &token.req_id, None::<&dyn EcssEnumeration>)?;
        sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn completion_failure<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        params: FailParams<E>,
    ) -> Result<(), SendStoredTmError<E>> {
        let tm = self.create_pus_verif_fail_tm(
            1,
            8,
            &token.req_id,
            None::<&dyn EcssEnumeration>,
            params.failure_code,
            params.failure_data,
        )?;
        params.sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }

    fn create_pus_verif_success_tm<E>(
        &mut self,
        service: u8,
        subservice: u8,
        req_id: &RequestId,
        step: Option<&(impl EcssEnumeration + ?Sized)>,
    ) -> Result<PusTm, SendStoredTmError<E>> {
        let mut source_data_len = size_of::<u32>();
        if let Some(step) = step {
            source_data_len += step.byte_width() as usize;
        }
        self.source_buffer_large_enough(source_data_len)?;
        let mut idx = 0;
        req_id.to_bytes(&mut self.source_data_buf[0..RequestId::SIZE_AS_BYTES]);
        idx += RequestId::SIZE_AS_BYTES;
        if let Some(step) = step {
            // Size check was done beforehand
            step.to_bytes(&mut self.source_data_buf[idx..idx + step.byte_width() as usize])
                .unwrap();
        }
        let mut sp_header = SpHeader::tm(self.apid, 0, 0).unwrap();
        self.time_stamper
            .write_to_bytes(self.time_stamp_buf.as_mut_slice())
            .map_err(|e| SendStoredTmError::TimeStampError(e))?;
        Ok(self.create_pus_verif_tm_base(service, subservice, &mut sp_header, source_data_len))
    }

    fn create_pus_verif_fail_tm<E>(
        &mut self,
        service: u8,
        subservice: u8,
        req_id: &RequestId,
        step: Option<&(impl EcssEnumeration + ?Sized)>,
        fail_code: &(impl EcssEnumeration + ?Sized),
        fail_data: &[u8],
    ) -> Result<PusTm, SendStoredTmError<E>> {
        let mut idx = 0;
        let mut source_data_len =
            RequestId::SIZE_AS_BYTES + fail_code.byte_width() as usize + fail_data.len();
        if let Some(step) = step {
            source_data_len += step.byte_width() as usize;
        }
        self.source_buffer_large_enough(source_data_len)?;
        req_id.to_bytes(&mut self.source_data_buf[0..RequestId::SIZE_AS_BYTES]);
        idx += RequestId::SIZE_AS_BYTES;
        if let Some(step) = step {
            // Size check done beforehand
            step.to_bytes(&mut self.source_data_buf[idx..idx + step.byte_width() as usize])
                .unwrap();
        }
        fail_code
            .to_bytes(&mut self.source_data_buf[idx..idx + fail_code.byte_width() as usize])
            .map_err(|e| SendStoredTmError::<E>::ToFromBytesError(e))?;
        idx += fail_code.byte_width() as usize;
        self.source_data_buf[idx..idx + fail_data.len()].copy_from_slice(fail_data);
        let mut sp_header = SpHeader::tm(self.apid, 0, 0).unwrap();
        self.time_stamper
            .write_to_bytes(self.time_stamp_buf.as_mut_slice())
            .map_err(|e| SendStoredTmError::<E>::TimeStampError(e))?;
        Ok(self.create_pus_verif_tm_base(service, subservice, &mut sp_header, source_data_len))
    }

    fn source_buffer_large_enough<E>(&self, len: usize) -> Result<(), SendStoredTmError<E>> {
        if len > self.source_data_buf.capacity() {
            return Err(SendStoredTmError::ToFromBytesError(
                ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                    found: self.source_data_buf.capacity(),
                    expected: len,
                }),
            ));
        }
        Ok(())
    }

    fn create_pus_verif_tm_base(
        &mut self,
        service: u8,
        subservice: u8,
        sp_header: &mut SpHeader,
        source_data_len: usize,
    ) -> PusTm {
        let tm_sec_header = PusTmSecondaryHeader::new(
            service,
            subservice,
            self.msg_count,
            self.dest_id,
            &self.time_stamp_buf,
        );
        PusTm::new(
            sp_header,
            tm_sec_header,
            Some(&self.source_data_buf[0..source_data_len]),
            true,
        )
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
