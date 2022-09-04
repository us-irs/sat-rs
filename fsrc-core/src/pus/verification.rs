use crate::pool::{LocalPool, StoreAddr, StoreError};
use alloc::vec;
use alloc::vec::Vec;
use core::mem::size_of;
use spacepackets::ecss::{EcssEnumeration, PusError};
use spacepackets::tc::PusTc;
use spacepackets::time::{CcsdsTimeProvider, TimestampError};
use spacepackets::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::{ByteConversionError, SizeMissmatch, SpHeader};
use spacepackets::{CcsdsPacket, PacketId, PacketSequenceCtrl};
use std::marker::PhantomData;

use alloc::sync::Arc;
#[cfg(feature = "std")]
use std::sync::{mpsc, Mutex};

#[cfg(feature = "std")]
use std::sync::mpsc::SendError;
use std::sync::MutexGuard;

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct RequestId {
    version_number: u8,
    packet_id: PacketId,
    psc: PacketSequenceCtrl,
}

#[derive(Debug, Clone)]
pub enum VerificationError<E> {
    SendError(E),
    TimeStampError(TimestampError),
    ByteConversionError(ByteConversionError),
    PusError(PusError),
    StoreError(StoreError),
}

#[derive(Debug, Clone)]
pub struct VerificationErrorWithToken<E, T>(VerificationError<E>, VerificationToken<T>);

impl RequestId {
    const SIZE_AS_BYTES: usize = size_of::<u32>();

    pub fn to_bytes(&self, buf: &mut [u8]) {
        let raw = ((self.version_number as u32) << 29)
            | ((self.packet_id.raw() as u32) << 16)
            | self.psc.raw() as u32;
        buf.copy_from_slice(raw.to_be_bytes().as_slice());
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < 4 {
            return None;
        }
        let raw = u32::from_be_bytes(buf[0..Self::SIZE_AS_BYTES].try_into().unwrap());
        Some(Self {
            version_number: ((raw >> 29) & 0b111) as u8,
            packet_id: PacketId::from(((raw >> 16) & 0xffff) as u16),
            psc: PacketSequenceCtrl::from((raw & 0xffff) as u16),
        })
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
    fn send_verification_tm(&mut self, tm: PusTm) -> Result<(), VerificationError<E>>;
}

#[cfg(feature = "std")]
pub struct StdVerifSender {
    pub ignore_poison_error: bool,
    tm_store: Arc<Mutex<LocalPool>>,
    tx: mpsc::Sender<StoreAddr>,
}

#[cfg(feature = "std")]
impl StdVerifSender {
    pub fn new(tm_store: Arc<Mutex<LocalPool>>, tx: mpsc::Sender<StoreAddr>) -> Self {
        Self {
            ignore_poison_error: true,
            tx,
            tm_store,
        }
    }
}

#[cfg(feature = "std")]
#[derive(Debug, Eq, PartialEq)]
pub enum StdVerifSenderError {
    PoisonError,
    SendError(SendError<StoreAddr>),
}

#[cfg(feature = "std")]
impl VerificationSender<StdVerifSenderError> for StdVerifSender {
    fn send_verification_tm(
        &mut self,
        tm: PusTm,
    ) -> Result<(), VerificationError<StdVerifSenderError>> {
        let operation = |mut mg: MutexGuard<LocalPool>| {
            let (addr, buf) = mg
                .free_element(tm.len_packed())
                .map_err(VerificationError::StoreError)?;
            tm.write_to(buf).map_err(VerificationError::PusError)?;
            self.tx
                .send(addr)
                .map_err(|e| VerificationError::SendError(StdVerifSenderError::SendError(e)))?;
            Ok(())
        };
        match self.tm_store.lock() {
            Ok(lock) => operation(lock),
            Err(poison_error) => {
                if self.ignore_poison_error {
                    operation(poison_error.into_inner())
                } else {
                    Err(VerificationError::SendError(
                        StdVerifSenderError::PoisonError,
                    ))
                }
            }
        }
    }
}

pub struct VerificationReporter {
    pub apid: u16,
    pub dest_id: u16,
    msg_count: u16,
    source_data_buf: Vec<u8>,
}

pub struct VerificationReporterCfg {
    pub apid: u16,
    pub dest_id: u16,
    pub step_field_width: u8,
    pub failure_code_field_width: u8,
    pub max_fail_data_len: usize,
    pub max_stamp_len: usize,
}

impl VerificationReporterCfg {
    pub fn new(time_stamper: impl CcsdsTimeProvider, apid: u16) -> Self {
        let max_stamp_len = time_stamper.len_as_bytes();
        Self {
            apid,
            dest_id: 0,
            step_field_width: size_of::<u8>() as u8,
            failure_code_field_width: size_of::<u16>() as u8,
            max_fail_data_len: 2 * size_of::<u32>(),
            max_stamp_len,
        }
    }
}

pub struct FailParams<'a> {
    time_stamp: &'a [u8],
    failure_code: &'a dyn EcssEnumeration,
    failure_data: &'a [u8],
}

impl<'a> FailParams<'a> {
    pub fn new(
        time_stamp: &'a [u8],
        failure_code: &'a impl EcssEnumeration,
        failure_data: &'a [u8],
    ) -> Self {
        Self {
            time_stamp,
            failure_code,
            failure_data,
        }
    }
}

pub struct FailParamsWithStep<'a> {
    bp: FailParams<'a>,
    step: &'a dyn EcssEnumeration,
}

impl<'a> FailParamsWithStep<'a> {
    pub fn new(
        time_stamp: &'a [u8],
        failure_code: &'a impl EcssEnumeration,
        failure_data: &'a [u8],
        step: &'a impl EcssEnumeration,
    ) -> Self {
        Self {
            bp: FailParams::new(time_stamp, failure_code, failure_data),
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
            source_data_buf: vec![
                0;
                RequestId::SIZE_AS_BYTES
                    + cfg.step_field_width as usize
                    + cfg.failure_code_field_width as usize
                    + cfg.max_fail_data_len
            ],
        }
    }

    pub fn add_tc(&mut self, pus_tc: &PusTc) -> VerificationToken<StateNone> {
        self.add_tc_with_req_id(RequestId::new(pus_tc))
    }

    pub fn add_tc_with_req_id(&mut self, req_id: RequestId) -> VerificationToken<StateNone> {
        VerificationToken::<StateNone>::new(req_id)
    }

    pub fn acceptance_success<E>(
        &mut self,
        token: VerificationToken<StateNone>,
        sender: &mut impl VerificationSender<E>,
        time_stamp: &[u8],
    ) -> Result<VerificationToken<StateAccepted>, VerificationErrorWithToken<E, StateNone>> {
        let tm = self
            .create_pus_verif_success_tm(
                1,
                1,
                &token.req_id,
                time_stamp,
                None::<&dyn EcssEnumeration>,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_verification_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
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
        params: FailParams,
    ) -> Result<(), VerificationErrorWithToken<E, StateNone>> {
        let tm = self
            .create_pus_verif_fail_tm(1, 2, &token.req_id, &params, None::<&dyn EcssEnumeration>)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_verification_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn start_success<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        sender: &mut impl VerificationSender<E>,
        time_stamp: &[u8],
    ) -> Result<VerificationToken<StateStarted>, VerificationErrorWithToken<E, StateAccepted>> {
        let tm = self
            .create_pus_verif_success_tm(
                1,
                3,
                &token.req_id,
                time_stamp,
                None::<&dyn EcssEnumeration>,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_verification_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        self.msg_count += 1;
        Ok(VerificationToken {
            state: PhantomData,
            req_id: token.req_id,
        })
    }

    pub fn start_failure<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        sender: &mut impl VerificationSender<E>,
        params: FailParams,
    ) -> Result<(), VerificationErrorWithToken<E, StateAccepted>> {
        let tm = self
            .create_pus_verif_fail_tm(1, 4, &token.req_id, &params, None::<&dyn EcssEnumeration>)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_verification_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn step_success<E>(
        &mut self,
        token: &VerificationToken<StateAccepted>,
        sender: &mut impl VerificationSender<E>,
        time_stamp: &[u8],
        step: impl EcssEnumeration,
    ) -> Result<(), VerificationError<E>> {
        let tm = self.create_pus_verif_success_tm(1, 5, &token.req_id, time_stamp, Some(&step))?;
        sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn step_failure<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        sender: &mut impl VerificationSender<E>,
        params: FailParamsWithStep,
    ) -> Result<(), VerificationErrorWithToken<E, StateAccepted>> {
        let tm = self
            .create_pus_verif_fail_tm(1, 6, &token.req_id, &params.bp, Some(params.step))
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_verification_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn completion_success<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        sender: &mut impl VerificationSender<E>,
        time_stamp: &[u8],
    ) -> Result<(), VerificationErrorWithToken<E, StateAccepted>> {
        let tm = self
            .create_pus_verif_success_tm(
                1,
                7,
                &token.req_id,
                time_stamp,
                None::<&dyn EcssEnumeration>,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_verification_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        self.msg_count += 1;
        Ok(())
    }

    pub fn completion_failure<E>(
        &mut self,
        token: VerificationToken<StateAccepted>,
        sender: &mut impl VerificationSender<E>,
        params: FailParams,
    ) -> Result<(), VerificationErrorWithToken<E, StateAccepted>> {
        let tm = self
            .create_pus_verif_fail_tm(1, 8, &token.req_id, &params, None::<&dyn EcssEnumeration>)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_verification_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        self.msg_count += 1;
        Ok(())
    }

    fn create_pus_verif_success_tm<'a, E>(
        &'a mut self,
        service: u8,
        subservice: u8,
        req_id: &RequestId,
        time_stamp: &'a [u8],
        step: Option<&(impl EcssEnumeration + ?Sized)>,
    ) -> Result<PusTm, VerificationError<E>> {
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
        Ok(self.create_pus_verif_tm_base(
            service,
            subservice,
            &mut sp_header,
            time_stamp,
            source_data_len,
        ))
    }

    fn create_pus_verif_fail_tm<'a, E>(
        &'a mut self,
        service: u8,
        subservice: u8,
        req_id: &RequestId,
        params: &'a FailParams,
        step: Option<&(impl EcssEnumeration + ?Sized)>,
    ) -> Result<PusTm, VerificationError<E>> {
        let mut idx = 0;
        let mut source_data_len = RequestId::SIZE_AS_BYTES
            + params.failure_code.byte_width() as usize
            + params.failure_data.len();
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
        params
            .failure_code
            .to_bytes(
                &mut self.source_data_buf[idx..idx + params.failure_code.byte_width() as usize],
            )
            .map_err(|e| VerificationError::<E>::ByteConversionError(e))?;
        idx += params.failure_code.byte_width() as usize;
        self.source_data_buf[idx..idx + params.failure_data.len()]
            .copy_from_slice(params.failure_data);
        let mut sp_header = SpHeader::tm(self.apid, 0, 0).unwrap();
        Ok(self.create_pus_verif_tm_base(
            service,
            subservice,
            &mut sp_header,
            params.time_stamp,
            source_data_len,
        ))
    }

    fn source_buffer_large_enough<E>(&self, len: usize) -> Result<(), VerificationError<E>> {
        if len > self.source_data_buf.capacity() {
            return Err(VerificationError::ByteConversionError(
                ByteConversionError::ToSliceTooSmall(SizeMissmatch {
                    found: self.source_data_buf.capacity(),
                    expected: len,
                }),
            ));
        }
        Ok(())
    }

    fn create_pus_verif_tm_base<'a>(
        &'a mut self,
        service: u8,
        subservice: u8,
        sp_header: &mut SpHeader,
        time_stamp: &'a [u8],
        source_data_len: usize,
    ) -> PusTm {
        let tm_sec_header = PusTmSecondaryHeader::new(
            service,
            subservice,
            self.msg_count,
            self.dest_id,
            time_stamp,
        );
        PusTm::new(
            sp_header,
            tm_sec_header,
            Some(&self.source_data_buf[0..source_data_len]),
            true,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VerificationToken<STATE> {
    state: PhantomData<STATE>,
    req_id: RequestId,
}

#[derive(Copy, Clone, Debug)]
pub struct StateNone;
#[derive(Copy, Clone, Debug)]
pub struct StateAccepted;
#[derive(Copy, Clone, Debug)]
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
    use crate::pus::verification::{
        RequestId, VerificationError, VerificationReporter, VerificationReporterCfg,
        VerificationSender,
    };
    use alloc::vec::Vec;
    use spacepackets::ecss::PusPacket;
    use spacepackets::tc::{PusTc, PusTcSecondaryHeader};
    use spacepackets::time::{CdsShortTimeProvider, TimeWriter};
    use spacepackets::tm::{PusTm, PusTmSecondaryHeaderT};
    use spacepackets::{CcsdsPacket, SpHeader};
    use std::collections::VecDeque;

    const TEST_APID: u16 = 0x02;

    struct TmInfo {
        pub subservice: u8,
        pub apid: u16,
        pub msg_counter: u16,
        pub dest_id: u16,
        pub time_stamp: [u8; 7],
        pub req_id: RequestId,
        pub additional_data: Option<Vec<u8>>,
    }

    #[derive(Default)]
    struct TestSender {
        pub service_queue: VecDeque<TmInfo>,
    }

    impl VerificationSender<()> for TestSender {
        fn send_verification_tm(&mut self, tm: PusTm) -> Result<(), VerificationError<()>> {
            assert_eq!(PusPacket::service(&tm), 1);
            assert!(tm.source_data().is_some());
            let mut time_stamp = [0; 7];
            time_stamp.clone_from_slice(tm.time_stamp());
            let src_data = tm.source_data().unwrap();
            assert!(src_data.len() >= 4);
            let req_id = RequestId::from_bytes(&src_data[0..RequestId::SIZE_AS_BYTES]).unwrap();
            let mut vec = None;
            if src_data.len() > 4 {
                let mut new_vec = Vec::new();
                new_vec.extend_from_slice(&src_data[RequestId::SIZE_AS_BYTES..]);
                vec = Some(new_vec);
            }
            self.service_queue.push_back(TmInfo {
                subservice: PusPacket::subservice(&tm),
                apid: tm.apid(),
                msg_counter: tm.msg_counter(),
                dest_id: tm.dest_id(),
                time_stamp,
                req_id,
                additional_data: vec,
            });
            Ok(())
        }
    }
    #[test]
    pub fn test_basic() {
        let time_stamper = CdsShortTimeProvider::default();
        let cfg = VerificationReporterCfg::new(time_stamper, 0x02);
        let mut reporter = VerificationReporter::new(cfg);
        let mut sph = SpHeader::tc(TEST_APID, 0x34, 0).unwrap();
        let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
        let pus_tc = PusTc::new(&mut sph, tc_header, None, true);
        let verif_token = reporter.add_tc(&pus_tc);
        let req_id = RequestId::new(&pus_tc);
        let mut stamp_buf = [0; 7];
        time_stamper.write_to_bytes(&mut stamp_buf).unwrap();
        let mut sender = TestSender::default();
        reporter
            .acceptance_success(verif_token, &mut sender, &stamp_buf)
            .expect("Sending acceptance success failed");
        assert_eq!(sender.service_queue.len(), 1);
        let info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info.subservice, 1);
        assert_eq!(info.time_stamp, [0; 7]);
        assert_eq!(info.dest_id, 0);
        assert_eq!(info.apid, TEST_APID);
        assert_eq!(info.msg_counter, 0);
        assert_eq!(info.additional_data, None);
        assert_eq!(info.req_id, req_id);
    }
}
