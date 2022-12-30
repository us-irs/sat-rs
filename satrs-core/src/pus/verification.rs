//! # PUS Verification Service 1 Module
//!
//! This module allows packaging and sending PUS Service 1 packets. It is conforming to section
//! 8 of the PUS standard ECSS-E-ST-70-41C.
//!
//! The core object to report TC verification progress is the [VerificationReporter]. It exposes
//! an API which uses type-state programming to avoid calling the verification steps in
//! an invalid order.
//!
//! # Examples
//!
//! Basic single-threaded example where a full success sequence for a given ping telecommand is
//! executed. Note that the verification part could also be done in a separate thread.
//!
//! ```
//! use std::sync::{Arc, mpsc, RwLock};
//! use std::time::Duration;
//! use satrs_core::pool::{LocalPool, PoolCfg, PoolProvider, SharedPool};
//! use satrs_core::pus::verification::{MpscVerifSender, VerificationReporterCfg, VerificationReporterWithSender};
//! use satrs_core::seq_count::SimpleSeqCountProvider;
//! use spacepackets::ecss::PusPacket;
//! use spacepackets::SpHeader;
//! use spacepackets::tc::{PusTc, PusTcSecondaryHeader};
//! use spacepackets::tm::PusTm;
//!
//! const EMPTY_STAMP: [u8; 7] = [0; 7];
//! const TEST_APID: u16 = 0x02;
//!
//! let pool_cfg = PoolCfg::new(vec![(10, 32), (10, 64), (10, 128), (10, 1024)]);
//! let shared_tm_pool: SharedPool = Arc::new(RwLock::new(Box::new(LocalPool::new(pool_cfg.clone()))));
//! let (verif_tx, verif_rx) = mpsc::channel();
//! let sender = MpscVerifSender::new(shared_tm_pool.clone(), verif_tx);
//! let cfg = VerificationReporterCfg::new(TEST_APID, Box::new(SimpleSeqCountProvider::default()), 1, 2, 8).unwrap();
//! let mut  reporter = VerificationReporterWithSender::new(&cfg , Box::new(sender));
//!
//! let mut sph = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
//! let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
//! let pus_tc_0 = PusTc::new(&mut sph, tc_header, None, true);
//! let init_token = reporter.add_tc(&pus_tc_0);
//!
//! // Complete success sequence for a telecommand
//! let accepted_token = reporter.acceptance_success(init_token, &EMPTY_STAMP).unwrap();
//! let started_token = reporter.start_success(accepted_token, &EMPTY_STAMP).unwrap();
//! reporter.completion_success(started_token, &EMPTY_STAMP).unwrap();
//!
//! // Verify it arrives correctly on receiver end
//! let mut tm_buf: [u8; 1024] = [0; 1024];
//! let mut packet_idx = 0;
//! while packet_idx < 3 {
//!     let addr = verif_rx.recv_timeout(Duration::from_millis(10)).unwrap();
//!     let tm_len;
//!     {
//!         let mut rg = shared_tm_pool.write().expect("Error locking shared pool");
//!         let store_guard = rg.read_with_guard(addr);
//!         let slice = store_guard.read().expect("Error reading TM slice");
//!         tm_len = slice.len();
//!         tm_buf[0..tm_len].copy_from_slice(slice);
//!     }
//!     let (pus_tm, _) = PusTm::from_bytes(&tm_buf[0..tm_len], 7)
//!        .expect("Error reading verification TM");
//!     if packet_idx == 0 {
//!         assert_eq!(pus_tm.subservice(), 1);
//!     } else if packet_idx == 1 {
//!         assert_eq!(pus_tm.subservice(), 3);
//!     } else if packet_idx == 2 {
//!         assert_eq!(pus_tm.subservice(), 7);
//!     }
//!     packet_idx += 1;
//! }
//! ```
//!
//! The [integration test](https://egit.irs.uni-stuttgart.de/rust/fsrc-launchpad/src/branch/main/fsrc-core/tests/verification_test.rs)
//! for the verification module contains examples how this module could be used in a more complex
//! context involving multiple threads
use crate::pus::{source_buffer_large_enough, EcssTmError, EcssTmSender};
use core::fmt::{Display, Formatter};
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::size_of;
#[cfg(feature = "alloc")]
use delegate::delegate;
use spacepackets::ecss::EcssEnumeration;
use spacepackets::tc::PusTc;
use spacepackets::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::{CcsdsPacket, PacketId, PacketSequenceCtrl};
use spacepackets::{SpHeader, MAX_APID};

pub use crate::seq_count::SimpleSeqCountProvider;

#[cfg(feature = "alloc")]
pub use alloc_mod::{
    VerificationReporterCfg, VerificationReporterWithBuf, VerificationReporterWithSender,
};

use crate::seq_count::SequenceCountProvider;
#[cfg(feature = "crossbeam")]
pub use stdmod::CrossbeamVerifSender;
#[cfg(feature = "std")]
pub use stdmod::{
    MpscVerifSender, SharedStdVerifReporterWithSender, StdVerifReporterWithSender,
    StdVerifSenderError,
};

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum Subservices {
    TmAcceptanceSuccess = 1,
    TmAcceptanceFailure = 2,
    TmStartSuccess = 3,
    TmStartFailure = 4,
    TmStepSuccess = 5,
    TmStepFailure = 6,
    TmCompletionSuccess = 7,
    TmCompletionFailure = 8,
}

impl From<Subservices> for u8 {
    fn from(enumeration: Subservices) -> Self {
        enumeration as u8
    }
}

/// This is a request identifier as specified in 5.4.11.2 c. of the PUS standard
/// This field equivalent to the first two bytes of the CCSDS space packet header.
#[derive(Debug, Eq, Copy, Clone)]
pub struct RequestId {
    version_number: u8,
    packet_id: PacketId,
    psc: PacketSequenceCtrl,
}

impl Display for RequestId {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#08x}", self.raw())
    }
}

impl Hash for RequestId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw().hash(state);
    }
}

// Implement manually to satisfy derive_hash_xor_eq lint
impl PartialEq for RequestId {
    fn eq(&self, other: &Self) -> bool {
        self.version_number == other.version_number
            && self.packet_id == other.packet_id
            && self.psc == other.psc
    }
}

impl RequestId {
    pub const SIZE_AS_BYTES: usize = size_of::<u32>();

    pub fn raw(&self) -> u32 {
        ((self.version_number as u32) << 29)
            | ((self.packet_id.raw() as u32) << 16)
            | self.psc.raw() as u32
    }

    pub fn to_bytes(&self, buf: &mut [u8]) {
        let raw = self.raw();
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
    /// This allows extracting the request ID from a given PUS telecommand.
    pub fn new(tc: &PusTc) -> Self {
        RequestId {
            version_number: tc.ccsds_version(),
            packet_id: tc.packet_id(),
            psc: tc.psc(),
        }
    }
}

/// If a verification operation fails, the passed token will be returned as well. This allows
/// re-trying the operation at a later point.
#[derive(Debug, Clone)]
pub struct VerificationErrorWithToken<E, T>(EcssTmError<E>, VerificationToken<T>);

/// Support token to allow type-state programming. This prevents calling the verification
/// steps in an invalid order.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct VerificationToken<STATE> {
    state: PhantomData<STATE>,
    req_id: RequestId,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TcStateNone;
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TcStateAccepted;
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TcStateStarted;

#[derive(Debug, Eq, PartialEq)]
pub enum TcStateToken {
    None(VerificationToken<TcStateNone>),
    Accepted(VerificationToken<TcStateAccepted>),
    Started(VerificationToken<TcStateStarted>),
}

impl From<VerificationToken<TcStateNone>> for TcStateToken {
    fn from(t: VerificationToken<TcStateNone>) -> Self {
        TcStateToken::None(t)
    }
}

impl From<VerificationToken<TcStateAccepted>> for TcStateToken {
    fn from(t: VerificationToken<TcStateAccepted>) -> Self {
        TcStateToken::Accepted(t)
    }
}

impl From<VerificationToken<TcStateStarted>> for TcStateToken {
    fn from(t: VerificationToken<TcStateStarted>) -> Self {
        TcStateToken::Started(t)
    }
}

impl<STATE> VerificationToken<STATE> {
    fn new(req_id: RequestId) -> VerificationToken<TcStateNone> {
        VerificationToken {
            state: PhantomData,
            req_id,
        }
    }

    pub fn req_id(&self) -> RequestId {
        self.req_id
    }
}

/// Composite helper struct to pass failure parameters to the [VerificationReporter]
pub struct FailParams<'a> {
    time_stamp: &'a [u8],
    failure_code: &'a dyn EcssEnumeration,
    failure_data: Option<&'a [u8]>,
}

impl<'a> FailParams<'a> {
    pub fn new(
        time_stamp: &'a [u8],
        failure_code: &'a impl EcssEnumeration,
        failure_data: Option<&'a [u8]>,
    ) -> Self {
        Self {
            time_stamp,
            failure_code,
            failure_data,
        }
    }
}

/// Composite helper struct to pass step failure parameters to the [VerificationReporter]
pub struct FailParamsWithStep<'a> {
    bp: FailParams<'a>,
    step: &'a dyn EcssEnumeration,
}

impl<'a> FailParamsWithStep<'a> {
    pub fn new(
        time_stamp: &'a [u8],
        step: &'a impl EcssEnumeration,
        failure_code: &'a impl EcssEnumeration,
        failure_data: Option<&'a [u8]>,
    ) -> Self {
        Self {
            bp: FailParams::new(time_stamp, failure_code, failure_data),
            step,
        }
    }
}

#[derive(Clone)]
pub struct VerificationReporterBasic {
    pub dest_id: u16,
    apid: u16,
}

impl VerificationReporterBasic {
    pub fn new(apid: u16) -> Option<Self> {
        if apid > MAX_APID {
            return None;
        }
        Some(Self { apid, dest_id: 0 })
    }

    pub fn set_apid(&mut self, apid: u16) -> bool {
        if apid > MAX_APID {
            return false;
        }
        self.apid = apid;
        true
    }

    pub fn apid(&self) -> u16 {
        self.apid
    }

    pub fn dest_id(&self) -> u16 {
        self.dest_id
    }

    pub fn set_dest_id(&mut self, dest_id: u16) {
        self.dest_id = dest_id;
    }

    /// Initialize verification handling by passing a TC reference. This returns a token required
    /// to call the acceptance functions
    pub fn add_tc(&mut self, pus_tc: &PusTc) -> VerificationToken<TcStateNone> {
        self.add_tc_with_req_id(RequestId::new(pus_tc))
    }

    /// Same as [Self::add_tc] but pass a request ID instead of the direct telecommand.
    /// This can be useful if the executing thread does not have full access to the telecommand.
    pub fn add_tc_with_req_id(&mut self, req_id: RequestId) -> VerificationToken<TcStateNone> {
        VerificationToken::<TcStateNone>::new(req_id)
    }

    /// Package and send a PUS TM\[1, 1\] packet, see 8.1.2.1 of the PUS standard
    pub fn acceptance_success<E>(
        &mut self,
        buf: &mut [u8],
        token: VerificationToken<TcStateNone>,
        sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
        seq_counter: &mut (impl SequenceCountProvider<u16> + ?Sized),
        time_stamp: &[u8],
    ) -> Result<VerificationToken<TcStateAccepted>, VerificationErrorWithToken<E, TcStateNone>>
    {
        let tm = self
            .create_pus_verif_success_tm(
                buf,
                Subservices::TmAcceptanceSuccess.into(),
                seq_counter.get(),
                &token.req_id,
                time_stamp,
                None::<&dyn EcssEnumeration>,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        seq_counter.increment();
        //seq_counter.get_and_increment()
        Ok(VerificationToken {
            state: PhantomData,
            req_id: token.req_id,
        })
    }

    /// Package and send a PUS TM\[1, 2\] packet, see 8.1.2.2 of the PUS standard
    pub fn acceptance_failure<E>(
        &mut self,
        buf: &mut [u8],
        token: VerificationToken<TcStateNone>,
        sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
        seq_counter: &mut (impl SequenceCountProvider<u16> + ?Sized),
        params: FailParams,
    ) -> Result<(), VerificationErrorWithToken<E, TcStateNone>> {
        let tm = self
            .create_pus_verif_fail_tm(
                buf,
                Subservices::TmAcceptanceFailure.into(),
                seq_counter.get(),
                &token.req_id,
                None::<&dyn EcssEnumeration>,
                &params,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        seq_counter.increment();
        Ok(())
    }

    /// Package and send a PUS TM\[1, 3\] packet, see 8.1.2.3 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::acceptance_success].
    pub fn start_success<E>(
        &mut self,
        buf: &mut [u8],
        token: VerificationToken<TcStateAccepted>,
        sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
        seq_counter: &mut (impl SequenceCountProvider<u16> + ?Sized),
        time_stamp: &[u8],
    ) -> Result<VerificationToken<TcStateStarted>, VerificationErrorWithToken<E, TcStateAccepted>>
    {
        let tm = self
            .create_pus_verif_success_tm(
                buf,
                Subservices::TmStartSuccess.into(),
                seq_counter.get(),
                &token.req_id,
                time_stamp,
                None::<&dyn EcssEnumeration>,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        seq_counter.increment();
        Ok(VerificationToken {
            state: PhantomData,
            req_id: token.req_id,
        })
    }

    /// Package and send a PUS TM\[1, 4\] packet, see 8.1.2.4 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::acceptance_success]. It consumes
    /// the token because verification handling is done.
    pub fn start_failure<E>(
        &mut self,
        buf: &mut [u8],
        token: VerificationToken<TcStateAccepted>,
        sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
        seq_counter: &mut (impl SequenceCountProvider<u16> + ?Sized),
        params: FailParams,
    ) -> Result<(), VerificationErrorWithToken<E, TcStateAccepted>> {
        let tm = self
            .create_pus_verif_fail_tm(
                buf,
                Subservices::TmStartFailure.into(),
                seq_counter.get(),
                &token.req_id,
                None::<&dyn EcssEnumeration>,
                &params,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        seq_counter.increment();
        Ok(())
    }

    /// Package and send a PUS TM\[1, 5\] packet, see 8.1.2.5 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::start_success].
    pub fn step_success<E>(
        &mut self,
        buf: &mut [u8],
        token: &VerificationToken<TcStateStarted>,
        sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
        seq_counter: &mut (impl SequenceCountProvider<u16> + ?Sized),
        time_stamp: &[u8],
        step: impl EcssEnumeration,
    ) -> Result<(), EcssTmError<E>> {
        let tm = self.create_pus_verif_success_tm(
            buf,
            Subservices::TmStepSuccess.into(),
            seq_counter.get(),
            &token.req_id,
            time_stamp,
            Some(&step),
        )?;
        sender.send_tm(tm)?;
        seq_counter.increment();
        Ok(())
    }

    /// Package and send a PUS TM\[1, 6\] packet, see 8.1.2.6 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
    /// token because verification handling is done.
    pub fn step_failure<E>(
        &mut self,
        buf: &mut [u8],
        token: VerificationToken<TcStateStarted>,
        sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
        seq_counter: &mut (impl SequenceCountProvider<u16> + ?Sized),
        params: FailParamsWithStep,
    ) -> Result<(), VerificationErrorWithToken<E, TcStateStarted>> {
        let tm = self
            .create_pus_verif_fail_tm(
                buf,
                Subservices::TmStepFailure.into(),
                seq_counter.get(),
                &token.req_id,
                Some(params.step),
                &params.bp,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        seq_counter.increment();
        Ok(())
    }

    /// Package and send a PUS TM\[1, 7\] packet, see 8.1.2.7 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
    /// token because verification handling is done.
    pub fn completion_success<E>(
        &mut self,
        buf: &mut [u8],
        token: VerificationToken<TcStateStarted>,
        sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
        seq_counter: &mut (impl SequenceCountProvider<u16> + ?Sized),
        time_stamp: &[u8],
    ) -> Result<(), VerificationErrorWithToken<E, TcStateStarted>> {
        let tm = self
            .create_pus_verif_success_tm(
                buf,
                Subservices::TmCompletionSuccess.into(),
                seq_counter.get(),
                &token.req_id,
                time_stamp,
                None::<&dyn EcssEnumeration>,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        seq_counter.increment();
        Ok(())
    }

    /// Package and send a PUS TM\[1, 8\] packet, see 8.1.2.8 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
    /// token because verification handling is done.
    pub fn completion_failure<E>(
        &mut self,
        buf: &mut [u8],
        token: VerificationToken<TcStateStarted>,
        sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
        seq_counter: &mut (impl SequenceCountProvider<u16> + ?Sized),
        params: FailParams,
    ) -> Result<(), VerificationErrorWithToken<E, TcStateStarted>> {
        let tm = self
            .create_pus_verif_fail_tm(
                buf,
                Subservices::TmCompletionFailure.into(),
                seq_counter.get(),
                &token.req_id,
                None::<&dyn EcssEnumeration>,
                &params,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        sender
            .send_tm(tm)
            .map_err(|e| VerificationErrorWithToken(e, token))?;
        seq_counter.increment();
        Ok(())
    }

    fn create_pus_verif_success_tm<'a, E>(
        &'a mut self,
        buf: &'a mut [u8],
        subservice: u8,
        msg_counter: u16,
        req_id: &RequestId,
        time_stamp: &'a [u8],
        step: Option<&(impl EcssEnumeration + ?Sized)>,
    ) -> Result<PusTm, EcssTmError<E>> {
        let mut source_data_len = size_of::<u32>();
        if let Some(step) = step {
            source_data_len += step.byte_width();
        }
        source_buffer_large_enough(buf.len(), source_data_len)?;
        let mut idx = 0;
        req_id.to_bytes(&mut buf[0..RequestId::SIZE_AS_BYTES]);
        idx += RequestId::SIZE_AS_BYTES;
        if let Some(step) = step {
            // Size check was done beforehand
            step.write_to_be_bytes(&mut buf[idx..idx + step.byte_width()])
                .unwrap();
        }
        let mut sp_header = SpHeader::tm_unseg(self.apid(), 0, 0).unwrap();
        Ok(self.create_pus_verif_tm_base(
            buf,
            subservice,
            msg_counter,
            &mut sp_header,
            time_stamp,
            source_data_len,
        ))
    }

    fn create_pus_verif_fail_tm<'a, E>(
        &'a mut self,
        buf: &'a mut [u8],
        subservice: u8,
        msg_counter: u16,
        req_id: &RequestId,
        step: Option<&(impl EcssEnumeration + ?Sized)>,
        params: &'a FailParams,
    ) -> Result<PusTm, EcssTmError<E>> {
        let mut idx = 0;
        let mut source_data_len = RequestId::SIZE_AS_BYTES + params.failure_code.byte_width();
        if let Some(step) = step {
            source_data_len += step.byte_width();
        }
        if let Some(failure_data) = params.failure_data {
            source_data_len += failure_data.len();
        }
        source_buffer_large_enough(buf.len(), source_data_len)?;
        req_id.to_bytes(&mut buf[0..RequestId::SIZE_AS_BYTES]);
        idx += RequestId::SIZE_AS_BYTES;
        if let Some(step) = step {
            // Size check done beforehand
            step.write_to_be_bytes(&mut buf[idx..idx + step.byte_width()])
                .unwrap();
            idx += step.byte_width();
        }
        params
            .failure_code
            .write_to_be_bytes(&mut buf[idx..idx + params.failure_code.byte_width()])?;
        idx += params.failure_code.byte_width();
        if let Some(failure_data) = params.failure_data {
            buf[idx..idx + failure_data.len()].copy_from_slice(failure_data);
        }
        let mut sp_header = SpHeader::tm_unseg(self.apid(), 0, 0).unwrap();
        Ok(self.create_pus_verif_tm_base(
            buf,
            subservice,
            msg_counter,
            &mut sp_header,
            params.time_stamp,
            source_data_len,
        ))
    }

    fn create_pus_verif_tm_base<'a>(
        &'a mut self,
        buf: &'a mut [u8],
        subservice: u8,
        msg_counter: u16,
        sp_header: &mut SpHeader,
        time_stamp: &'a [u8],
        source_data_len: usize,
    ) -> PusTm {
        let tm_sec_header =
            PusTmSecondaryHeader::new(1, subservice, msg_counter, self.dest_id, time_stamp);
        PusTm::new(
            sp_header,
            tm_sec_header,
            Some(&buf[0..source_data_len]),
            true,
        )
    }
}

#[cfg(feature = "alloc")]
mod alloc_mod {
    use super::*;
    use alloc::boxed::Box;
    use alloc::vec;
    use alloc::vec::Vec;

    #[derive(Clone)]
    pub struct VerificationReporterCfg {
        apid: u16,
        seq_counter: Box<dyn SequenceCountProvider<u16> + Send>,
        pub step_field_width: usize,
        pub fail_code_field_width: usize,
        pub max_fail_data_len: usize,
    }

    impl VerificationReporterCfg {
        pub fn new(
            apid: u16,
            seq_counter: Box<dyn SequenceCountProvider<u16> + Send>,
            step_field_width: usize,
            fail_code_field_width: usize,
            max_fail_data_len: usize,
        ) -> Option<Self> {
            if apid > MAX_APID {
                return None;
            }
            Some(Self {
                apid,
                seq_counter,
                step_field_width,
                fail_code_field_width,
                max_fail_data_len,
            })
        }
    }

    /// Primary verification handler. It provides an API to send PUS 1 verification telemetry packets
    /// and verify the various steps of telecommand handling as specified in the PUS standard.
    #[derive(Clone)]
    pub struct VerificationReporterWithBuf {
        source_data_buf: Vec<u8>,
        seq_counter: Box<dyn SequenceCountProvider<u16> + Send + 'static>,
        pub reporter: VerificationReporterBasic,
    }

    impl VerificationReporterWithBuf {
        pub fn new(cfg: &VerificationReporterCfg) -> Self {
            let reporter = VerificationReporterBasic::new(cfg.apid).unwrap();
            Self {
                source_data_buf: vec![
                    0;
                    RequestId::SIZE_AS_BYTES
                        + cfg.step_field_width
                        + cfg.fail_code_field_width
                        + cfg.max_fail_data_len
                ],
                seq_counter: cfg.seq_counter.clone(),
                reporter,
            }
        }

        delegate!(
            to self.reporter {
                pub fn set_apid(&mut self, apid: u16) -> bool;
                pub fn apid(&self) -> u16;
                pub fn add_tc(&mut self, pus_tc: &PusTc) -> VerificationToken<TcStateNone>;
                pub fn add_tc_with_req_id(&mut self, req_id: RequestId) -> VerificationToken<TcStateNone>;
                pub fn dest_id(&self) -> u16;
                pub fn set_dest_id(&mut self, dest_id: u16);
            }
        );

        pub fn allowed_source_data_len(&self) -> usize {
            self.source_data_buf.capacity()
        }

        /// Package and send a PUS TM\[1, 1\] packet, see 8.1.2.1 of the PUS standard
        pub fn acceptance_success<E>(
            &mut self,
            token: VerificationToken<TcStateNone>,
            sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
            time_stamp: &[u8],
        ) -> Result<VerificationToken<TcStateAccepted>, VerificationErrorWithToken<E, TcStateNone>>
        {
            self.reporter.acceptance_success(
                self.source_data_buf.as_mut_slice(),
                token,
                sender,
                self.seq_counter.as_mut(),
                time_stamp,
            )
        }

        /// Package and send a PUS TM\[1, 2\] packet, see 8.1.2.2 of the PUS standard
        pub fn acceptance_failure<E>(
            &mut self,
            token: VerificationToken<TcStateNone>,
            sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
            params: FailParams,
        ) -> Result<(), VerificationErrorWithToken<E, TcStateNone>> {
            self.reporter.acceptance_failure(
                self.source_data_buf.as_mut_slice(),
                token,
                sender,
                self.seq_counter.as_mut(),
                params,
            )
        }

        /// Package and send a PUS TM\[1, 3\] packet, see 8.1.2.3 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::acceptance_success].
        pub fn start_success<E>(
            &mut self,
            token: VerificationToken<TcStateAccepted>,
            sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
            time_stamp: &[u8],
        ) -> Result<VerificationToken<TcStateStarted>, VerificationErrorWithToken<E, TcStateAccepted>>
        {
            self.reporter.start_success(
                self.source_data_buf.as_mut_slice(),
                token,
                sender,
                self.seq_counter.as_mut(),
                time_stamp,
            )
        }

        /// Package and send a PUS TM\[1, 4\] packet, see 8.1.2.4 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::acceptance_success]. It consumes
        /// the token because verification handling is done.
        pub fn start_failure<E>(
            &mut self,
            token: VerificationToken<TcStateAccepted>,
            sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
            params: FailParams,
        ) -> Result<(), VerificationErrorWithToken<E, TcStateAccepted>> {
            self.reporter.start_failure(
                self.source_data_buf.as_mut_slice(),
                token,
                sender,
                self.seq_counter.as_mut(),
                params,
            )
        }

        /// Package and send a PUS TM\[1, 5\] packet, see 8.1.2.5 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::start_success].
        pub fn step_success<E>(
            &mut self,
            token: &VerificationToken<TcStateStarted>,
            sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
            time_stamp: &[u8],
            step: impl EcssEnumeration,
        ) -> Result<(), EcssTmError<E>> {
            self.reporter.step_success(
                self.source_data_buf.as_mut_slice(),
                token,
                sender,
                self.seq_counter.as_mut(),
                time_stamp,
                step,
            )
        }

        /// Package and send a PUS TM\[1, 6\] packet, see 8.1.2.6 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
        /// token because verification handling is done.
        pub fn step_failure<E>(
            &mut self,
            token: VerificationToken<TcStateStarted>,
            sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
            params: FailParamsWithStep,
        ) -> Result<(), VerificationErrorWithToken<E, TcStateStarted>> {
            self.reporter.step_failure(
                self.source_data_buf.as_mut_slice(),
                token,
                sender,
                self.seq_counter.as_mut(),
                params,
            )
        }

        /// Package and send a PUS TM\[1, 7\] packet, see 8.1.2.7 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
        /// token because verification handling is done.
        pub fn completion_success<E>(
            &mut self,
            token: VerificationToken<TcStateStarted>,
            sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
            time_stamp: &[u8],
        ) -> Result<(), VerificationErrorWithToken<E, TcStateStarted>> {
            self.reporter.completion_success(
                self.source_data_buf.as_mut_slice(),
                token,
                sender,
                self.seq_counter.as_mut(),
                time_stamp,
            )
        }

        /// Package and send a PUS TM\[1, 8\] packet, see 8.1.2.8 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
        /// token because verification handling is done.
        pub fn completion_failure<E>(
            &mut self,
            token: VerificationToken<TcStateStarted>,
            sender: &mut (impl EcssTmSender<Error = E> + ?Sized),
            params: FailParams,
        ) -> Result<(), VerificationErrorWithToken<E, TcStateStarted>> {
            self.reporter.completion_failure(
                self.source_data_buf.as_mut_slice(),
                token,
                sender,
                self.seq_counter.as_mut(),
                params,
            )
        }
    }

    /// Helper object which caches the sender passed as a trait object. Provides the same
    /// API as [VerificationReporter] but without the explicit sender arguments.
    #[derive(Clone)]
    pub struct VerificationReporterWithSender<E> {
        pub reporter: VerificationReporterWithBuf,
        pub sender: Box<dyn EcssTmSender<Error = E>>,
    }

    impl<E: 'static> VerificationReporterWithSender<E> {
        pub fn new(
            cfg: &VerificationReporterCfg,
            sender: Box<dyn EcssTmSender<Error = E>>,
        ) -> Self {
            let reporter = VerificationReporterWithBuf::new(cfg);
            Self::new_from_reporter(reporter, sender)
        }

        pub fn new_from_reporter(
            reporter: VerificationReporterWithBuf,
            sender: Box<dyn EcssTmSender<Error = E>>,
        ) -> Self {
            Self { reporter, sender }
        }

        delegate! {
            to self.reporter {
                pub fn set_apid(&mut self, apid: u16) -> bool;
                pub fn apid(&self) -> u16;
                pub fn add_tc(&mut self, pus_tc: &PusTc) -> VerificationToken<TcStateNone>;
                pub fn add_tc_with_req_id(&mut self, req_id: RequestId) -> VerificationToken<TcStateNone>;
                pub fn dest_id(&self) -> u16;
                pub fn set_dest_id(&mut self, dest_id: u16);
            }
        }

        pub fn acceptance_success(
            &mut self,
            token: VerificationToken<TcStateNone>,
            time_stamp: &[u8],
        ) -> Result<VerificationToken<TcStateAccepted>, VerificationErrorWithToken<E, TcStateNone>>
        {
            self.reporter
                .acceptance_success(token, self.sender.as_mut(), time_stamp)
        }

        pub fn acceptance_failure(
            &mut self,
            token: VerificationToken<TcStateNone>,
            params: FailParams,
        ) -> Result<(), VerificationErrorWithToken<E, TcStateNone>> {
            self.reporter
                .acceptance_failure(token, self.sender.as_mut(), params)
        }

        pub fn start_success(
            &mut self,
            token: VerificationToken<TcStateAccepted>,
            time_stamp: &[u8],
        ) -> Result<VerificationToken<TcStateStarted>, VerificationErrorWithToken<E, TcStateAccepted>>
        {
            self.reporter
                .start_success(token, self.sender.as_mut(), time_stamp)
        }

        pub fn start_failure(
            &mut self,
            token: VerificationToken<TcStateAccepted>,
            params: FailParams,
        ) -> Result<(), VerificationErrorWithToken<E, TcStateAccepted>> {
            self.reporter
                .start_failure(token, self.sender.as_mut(), params)
        }

        pub fn step_success(
            &mut self,
            token: &VerificationToken<TcStateStarted>,
            time_stamp: &[u8],
            step: impl EcssEnumeration,
        ) -> Result<(), EcssTmError<E>> {
            self.reporter
                .step_success(token, self.sender.as_mut(), time_stamp, step)
        }

        pub fn step_failure(
            &mut self,
            token: VerificationToken<TcStateStarted>,
            params: FailParamsWithStep,
        ) -> Result<(), VerificationErrorWithToken<E, TcStateStarted>> {
            self.reporter
                .step_failure(token, self.sender.as_mut(), params)
        }

        pub fn completion_success(
            &mut self,
            token: VerificationToken<TcStateStarted>,
            time_stamp: &[u8],
        ) -> Result<(), VerificationErrorWithToken<E, TcStateStarted>> {
            self.reporter
                .completion_success(token, self.sender.as_mut(), time_stamp)
        }

        pub fn completion_failure(
            &mut self,
            token: VerificationToken<TcStateStarted>,
            params: FailParams,
        ) -> Result<(), VerificationErrorWithToken<E, TcStateStarted>> {
            self.reporter
                .completion_failure(token, self.sender.as_mut(), params)
        }
    }
}

#[cfg(feature = "std")]
mod stdmod {
    use super::alloc_mod::VerificationReporterWithSender;
    use super::*;
    use crate::pool::{ShareablePoolProvider, SharedPool, StoreAddr, StoreError};
    use delegate::delegate;
    use spacepackets::tm::PusTm;
    use std::sync::{mpsc, Arc, Mutex, RwLockWriteGuard};

    pub type StdVerifReporterWithSender = VerificationReporterWithSender<StdVerifSenderError>;
    pub type SharedStdVerifReporterWithSender = Arc<Mutex<StdVerifReporterWithSender>>;

    #[derive(Debug, Eq, PartialEq, Clone)]
    pub enum StdVerifSenderError {
        PoisonError,
        StoreError(StoreError),
        RxDisconnected(StoreAddr),
    }

    impl From<StoreError> for StdVerifSenderError {
        fn from(e: StoreError) -> Self {
            StdVerifSenderError::StoreError(e)
        }
    }

    impl From<StoreError> for EcssTmError<StdVerifSenderError> {
        fn from(e: StoreError) -> Self {
            EcssTmError::SendError(e.into())
        }
    }

    trait SendBackend: Send {
        fn send(&self, addr: StoreAddr) -> Result<(), StoreAddr>;
    }

    #[derive(Clone)]
    struct StdSenderBase<S> {
        pub ignore_poison_error: bool,
        tm_store: SharedPool,
        tx: S,
    }

    impl<S: SendBackend> StdSenderBase<S> {
        pub fn new(tm_store: SharedPool, tx: S) -> Self {
            Self {
                ignore_poison_error: false,
                tm_store,
                tx,
            }
        }
    }

    unsafe impl<S: Sync> Sync for StdSenderBase<S> {}
    unsafe impl<S: Send> Send for StdSenderBase<S> {}

    impl SendBackend for mpsc::Sender<StoreAddr> {
        fn send(&self, addr: StoreAddr) -> Result<(), StoreAddr> {
            self.send(addr).map_err(|_| addr)
        }
    }

    #[derive(Clone)]
    pub struct MpscVerifSender {
        base: StdSenderBase<mpsc::Sender<StoreAddr>>,
    }

    /// Verification sender with a [mpsc::Sender] backend.
    /// It implements the [EcssTmSender] trait to be used as PUS Verification TM sender.
    impl MpscVerifSender {
        pub fn new(tm_store: SharedPool, tx: mpsc::Sender<StoreAddr>) -> Self {
            Self {
                base: StdSenderBase::new(tm_store, tx),
            }
        }
    }

    //noinspection RsTraitImplementation
    impl EcssTmSender for MpscVerifSender {
        type Error = StdVerifSenderError;

        delegate!(
            to self.base {
                fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmError<StdVerifSenderError>>;
            }
        );
    }
    unsafe impl Sync for MpscVerifSender {}
    unsafe impl Send for MpscVerifSender {}

    impl SendBackend for crossbeam_channel::Sender<StoreAddr> {
        fn send(&self, addr: StoreAddr) -> Result<(), StoreAddr> {
            self.send(addr).map_err(|_| addr)
        }
    }

    /// Verification sender with a [crossbeam_channel::Sender] backend.
    /// It implements the [EcssTmSender] trait to be used as PUS Verification TM sender
    #[cfg(feature = "crossbeam")]
    #[derive(Clone)]
    pub struct CrossbeamVerifSender {
        base: StdSenderBase<crossbeam_channel::Sender<StoreAddr>>,
    }

    #[cfg(feature = "crossbeam")]
    impl CrossbeamVerifSender {
        pub fn new(tm_store: SharedPool, tx: crossbeam_channel::Sender<StoreAddr>) -> Self {
            Self {
                base: StdSenderBase::new(tm_store, tx),
            }
        }
    }

    //noinspection RsTraitImplementation
    #[cfg(feature = "crossbeam")]
    impl EcssTmSender for CrossbeamVerifSender {
        type Error = StdVerifSenderError;

        delegate!(
            to self.base {
                fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmError<StdVerifSenderError>>;
            }
        );
    }

    // TODO: Are those really necessary? Check with test..
    #[cfg(feature = "crossbeam")]
    unsafe impl Sync for CrossbeamVerifSender {}
    #[cfg(feature = "crossbeam")]
    unsafe impl Send for CrossbeamVerifSender {}

    impl<S: SendBackend + Clone + 'static> EcssTmSender for StdSenderBase<S> {
        type Error = StdVerifSenderError;
        fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmError<Self::Error>> {
            let operation = |mut mg: RwLockWriteGuard<ShareablePoolProvider>| {
                let (addr, buf) = mg.free_element(tm.len_packed())?;
                tm.write_to_bytes(buf).map_err(EcssTmError::PusError)?;
                drop(mg);
                self.tx.send(addr).map_err(|_| {
                    EcssTmError::SendError(StdVerifSenderError::RxDisconnected(addr))
                })?;
                Ok(())
            };
            match self.tm_store.write() {
                Ok(lock) => operation(lock),
                Err(poison_error) => {
                    if self.ignore_poison_error {
                        operation(poison_error.into_inner())
                    } else {
                        Err(EcssTmError::SendError(StdVerifSenderError::PoisonError))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pus::tests::CommonTmInfo;
    use crate::pus::verification::{
        EcssTmError, EcssTmSender, FailParams, FailParamsWithStep, RequestId, TcStateNone,
        VerificationReporterCfg, VerificationReporterWithBuf, VerificationReporterWithSender,
        VerificationToken,
    };
    use crate::seq_count::SimpleSeqCountProvider;
    use alloc::boxed::Box;
    use alloc::format;
    use spacepackets::ecss::{EcssEnumU16, EcssEnumU32, EcssEnumU8, EcssEnumeration, PusPacket};
    use spacepackets::tc::{PusTc, PusTcSecondaryHeader};
    use spacepackets::tm::PusTm;
    use spacepackets::{ByteConversionError, SpHeader};
    use std::collections::VecDeque;
    use std::vec::Vec;

    const TEST_APID: u16 = 0x02;
    const EMPTY_STAMP: [u8; 7] = [0; 7];

    #[derive(Debug, Eq, PartialEq, Clone)]
    struct TmInfo {
        pub common: CommonTmInfo,
        pub req_id: RequestId,
        pub additional_data: Option<Vec<u8>>,
    }

    #[derive(Default, Clone)]
    struct TestSender {
        pub service_queue: VecDeque<TmInfo>,
    }

    impl EcssTmSender for TestSender {
        type Error = ();
        fn send_tm(&mut self, tm: PusTm) -> Result<(), EcssTmError<Self::Error>> {
            assert_eq!(PusPacket::service(&tm), 1);
            assert!(tm.source_data().is_some());
            let mut time_stamp = [0; 7];
            time_stamp.clone_from_slice(&tm.time_stamp()[0..7]);
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
                common: CommonTmInfo::new_from_tm(&tm),
                req_id,
                additional_data: vec,
            });
            Ok(())
        }
    }

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    struct DummyError {}
    #[derive(Default, Clone)]
    struct FallibleSender {}

    impl EcssTmSender for FallibleSender {
        type Error = DummyError;
        fn send_tm(&mut self, _: PusTm) -> Result<(), EcssTmError<DummyError>> {
            Err(EcssTmError::SendError(DummyError {}))
        }
    }

    struct TestBase<'a> {
        vr: VerificationReporterWithBuf,
        #[allow(dead_code)]
        tc: PusTc<'a>,
    }

    impl<'a> TestBase<'a> {
        fn rep(&mut self) -> &mut VerificationReporterWithBuf {
            &mut self.vr
        }
    }
    struct TestBaseWithHelper<'a, E> {
        helper: VerificationReporterWithSender<E>,
        #[allow(dead_code)]
        tc: PusTc<'a>,
    }

    impl<'a, E> TestBaseWithHelper<'a, E> {
        fn rep(&mut self) -> &mut VerificationReporterWithBuf {
            &mut self.helper.reporter
        }
    }

    fn base_reporter() -> VerificationReporterWithBuf {
        let cfg = VerificationReporterCfg::new(
            TEST_APID,
            Box::new(SimpleSeqCountProvider::default()),
            1,
            2,
            8,
        )
        .unwrap();
        VerificationReporterWithBuf::new(&cfg)
    }

    fn base_tc_init(app_data: Option<&[u8]>) -> (PusTc, RequestId) {
        let mut sph = SpHeader::tc_unseg(TEST_APID, 0x34, 0).unwrap();
        let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
        let pus_tc = PusTc::new(&mut sph, tc_header, app_data, true);
        let req_id = RequestId::new(&pus_tc);
        (pus_tc, req_id)
    }

    fn base_init(api_sel: bool) -> (TestBase<'static>, VerificationToken<TcStateNone>) {
        let mut reporter = base_reporter();
        let (tc, req_id) = base_tc_init(None);
        let init_tok;
        if api_sel {
            init_tok = reporter.add_tc_with_req_id(req_id);
        } else {
            init_tok = reporter.add_tc(&tc);
        }
        (TestBase { vr: reporter, tc }, init_tok)
    }

    fn base_with_helper_init() -> (
        TestBaseWithHelper<'static, ()>,
        VerificationToken<TcStateNone>,
    ) {
        let mut reporter = base_reporter();
        let (tc, _) = base_tc_init(None);
        let init_tok = reporter.add_tc(&tc);
        let sender = TestSender::default();
        let helper = VerificationReporterWithSender::new_from_reporter(reporter, Box::new(sender));
        (TestBaseWithHelper { helper, tc }, init_tok)
    }

    fn acceptance_check(sender: &mut TestSender, req_id: &RequestId) {
        let cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 1,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: None,
            req_id: req_id.clone(),
        };
        assert_eq!(sender.service_queue.len(), 1);
        let info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_state() {
        let (mut b, _) = base_init(false);
        assert_eq!(b.vr.apid(), TEST_APID);
        b.vr.set_apid(TEST_APID + 1);
        assert_eq!(b.vr.apid(), TEST_APID + 1);
    }

    #[test]
    fn test_basic_acceptance_success() {
        let (mut b, tok) = base_init(false);
        let mut sender = TestSender::default();
        b.vr.acceptance_success(tok, &mut sender, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        acceptance_check(&mut sender, &tok.req_id);
    }

    #[test]
    fn test_basic_acceptance_success_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        b.helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let sender: &mut TestSender = b.helper.sender.downcast_mut().unwrap();
        acceptance_check(sender, &tok.req_id);
    }

    #[test]
    fn test_acceptance_send_fails() {
        let (mut b, tok) = base_init(false);
        let mut faulty_sender = FallibleSender::default();
        let res =
            b.vr.acceptance_success(tok, &mut faulty_sender, &EMPTY_STAMP);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.1, tok);
        match err.0 {
            EcssTmError::SendError(e) => {
                assert_eq!(e, DummyError {})
            }
            _ => panic!("{}", format!("Unexpected error {:?}", err.0)),
        }
    }

    fn acceptance_fail_check(sender: &mut TestSender, req_id: RequestId, stamp_buf: [u8; 7]) {
        let cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 2,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 5,
                time_stamp: stamp_buf,
            },
            additional_data: Some([0, 2].to_vec()),
            req_id,
        };
        assert_eq!(sender.service_queue.len(), 1);
        let info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_basic_acceptance_failure() {
        let (mut b, tok) = base_init(true);
        b.rep().reporter.dest_id = 5;
        let stamp_buf = [1, 2, 3, 4, 5, 6, 7];
        let mut sender = TestSender::default();
        let fail_code = EcssEnumU16::new(2);
        let fail_params = FailParams::new(stamp_buf.as_slice(), &fail_code, None);
        b.vr.acceptance_failure(tok, &mut sender, fail_params)
            .expect("Sending acceptance success failed");
        acceptance_fail_check(&mut sender, tok.req_id, stamp_buf);
    }

    #[test]
    fn test_basic_acceptance_failure_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        b.rep().reporter.dest_id = 5;
        let stamp_buf = [1, 2, 3, 4, 5, 6, 7];
        let fail_code = EcssEnumU16::new(2);
        let fail_params = FailParams::new(stamp_buf.as_slice(), &fail_code, None);
        b.helper
            .acceptance_failure(tok, fail_params)
            .expect("Sending acceptance success failed");
        let sender: &mut TestSender = b.helper.sender.downcast_mut().unwrap();
        acceptance_fail_check(sender, tok.req_id, stamp_buf);
    }

    #[test]
    fn test_acceptance_fail_data_too_large() {
        let (mut b, tok) = base_with_helper_init();
        b.rep().reporter.dest_id = 5;
        let stamp_buf = [1, 2, 3, 4, 5, 6, 7];
        let fail_code = EcssEnumU16::new(2);
        let fail_data: [u8; 16] = [0; 16];
        // 4 req ID + 1 byte step + 2 byte error code + 8 byte fail data
        assert_eq!(b.rep().allowed_source_data_len(), 15);
        let fail_params =
            FailParams::new(stamp_buf.as_slice(), &fail_code, Some(fail_data.as_slice()));
        let res = b.helper.acceptance_failure(tok, fail_params);
        assert!(res.is_err());
        let err_with_token = res.unwrap_err();
        assert_eq!(err_with_token.1, tok);
        match err_with_token.0 {
            EcssTmError::ByteConversionError(e) => match e {
                ByteConversionError::ToSliceTooSmall(missmatch) => {
                    assert_eq!(
                        missmatch.expected,
                        fail_data.len() + RequestId::SIZE_AS_BYTES + fail_code.byte_width()
                    );
                    assert_eq!(missmatch.found, b.rep().allowed_source_data_len());
                }
                _ => {
                    panic!("{}", format!("Unexpected error {:?}", e))
                }
            },
            _ => {
                panic!("{}", format!("Unexpected error {:?}", err_with_token.0))
            }
        }
    }

    #[test]
    fn test_basic_acceptance_failure_with_fail_data() {
        let (mut b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let fail_code = EcssEnumU8::new(10);
        let fail_data = EcssEnumU32::new(12);
        let mut fail_data_raw = [0; 4];
        fail_data.write_to_be_bytes(&mut fail_data_raw).unwrap();
        let fail_params = FailParams::new(&EMPTY_STAMP, &fail_code, Some(fail_data_raw.as_slice()));
        b.vr.acceptance_failure(tok, &mut sender, fail_params)
            .expect("Sending acceptance success failed");
        let cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 2,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([10, 0, 0, 0, 12].to_vec()),
            req_id: tok.req_id,
        };
        assert_eq!(sender.service_queue.len(), 1);
        let info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    fn start_fail_check(sender: &mut TestSender, req_id: RequestId, fail_data_raw: [u8; 4]) {
        assert_eq!(sender.service_queue.len(), 2);
        let mut cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 1,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: None,
            req_id,
        };
        let mut info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 4,
                apid: TEST_APID,
                msg_counter: 1,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([&[22], fail_data_raw.as_slice()].concat().to_vec()),
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_start_failure() {
        let (mut b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let fail_code = EcssEnumU8::new(22);
        let fail_data: i32 = -12;
        let mut fail_data_raw = [0; 4];
        fail_data_raw.copy_from_slice(fail_data.to_be_bytes().as_slice());
        let fail_params = FailParams::new(&EMPTY_STAMP, &fail_code, Some(fail_data_raw.as_slice()));

        let accepted_token =
            b.vr.acceptance_success(tok, &mut sender, &EMPTY_STAMP)
                .expect("Sending acceptance success failed");
        let empty =
            b.vr.start_failure(accepted_token, &mut sender, fail_params)
                .expect("Start failure failure");
        assert_eq!(empty, ());
        start_fail_check(&mut sender, tok.req_id, fail_data_raw);
    }

    #[test]
    fn test_start_failure_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        let fail_code = EcssEnumU8::new(22);
        let fail_data: i32 = -12;
        let mut fail_data_raw = [0; 4];
        fail_data_raw.copy_from_slice(fail_data.to_be_bytes().as_slice());
        let fail_params = FailParams::new(&EMPTY_STAMP, &fail_code, Some(fail_data_raw.as_slice()));

        let accepted_token = b
            .helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let empty = b
            .helper
            .start_failure(accepted_token, fail_params)
            .expect("Start failure failure");
        assert_eq!(empty, ());
        let sender: &mut TestSender = b.helper.sender.downcast_mut().unwrap();
        start_fail_check(sender, tok.req_id, fail_data_raw);
    }

    fn step_success_check(sender: &mut TestSender, req_id: RequestId) {
        let mut cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 1,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: None,
            req_id,
        };
        let mut info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 3,
                apid: TEST_APID,
                msg_counter: 1,
                dest_id: 0,
                time_stamp: [0, 1, 0, 1, 0, 1, 0],
            },
            additional_data: None,
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 5,
                apid: TEST_APID,
                msg_counter: 2,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([0].to_vec()),
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 5,
                apid: TEST_APID,
                msg_counter: 3,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([1].to_vec()),
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_steps_success() {
        let (mut b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let accepted_token = b
            .rep()
            .acceptance_success(tok, &mut sender, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let started_token = b
            .rep()
            .start_success(accepted_token, &mut sender, &[0, 1, 0, 1, 0, 1, 0])
            .expect("Sending start success failed");
        let mut empty = b
            .rep()
            .step_success(
                &started_token,
                &mut sender,
                &EMPTY_STAMP,
                EcssEnumU8::new(0),
            )
            .expect("Sending step 0 success failed");
        assert_eq!(empty, ());
        empty =
            b.vr.step_success(
                &started_token,
                &mut sender,
                &EMPTY_STAMP,
                EcssEnumU8::new(1),
            )
            .expect("Sending step 1 success failed");
        assert_eq!(empty, ());
        assert_eq!(sender.service_queue.len(), 4);
        step_success_check(&mut sender, tok.req_id);
    }

    #[test]
    fn test_steps_success_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        let accepted_token = b
            .helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let started_token = b
            .helper
            .start_success(accepted_token, &[0, 1, 0, 1, 0, 1, 0])
            .expect("Sending start success failed");
        let mut empty = b
            .helper
            .step_success(&started_token, &EMPTY_STAMP, EcssEnumU8::new(0))
            .expect("Sending step 0 success failed");
        assert_eq!(empty, ());
        empty = b
            .helper
            .step_success(&started_token, &EMPTY_STAMP, EcssEnumU8::new(1))
            .expect("Sending step 1 success failed");
        assert_eq!(empty, ());
        let sender: &mut TestSender = b.helper.sender.downcast_mut().unwrap();
        assert_eq!(sender.service_queue.len(), 4);
        step_success_check(sender, tok.req_id);
    }

    fn check_step_failure(sender: &mut TestSender, req_id: RequestId, fail_data_raw: [u8; 4]) {
        assert_eq!(sender.service_queue.len(), 4);
        let mut cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 1,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: None,
            req_id,
        };
        let mut info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 3,
                apid: TEST_APID,
                msg_counter: 1,
                dest_id: 0,
                time_stamp: [0, 1, 0, 1, 0, 1, 0],
            },
            additional_data: None,
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 5,
                apid: TEST_APID,
                msg_counter: 2,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([0].to_vec()),
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 6,
                apid: TEST_APID,
                msg_counter: 3,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some(
                [
                    [1].as_slice(),
                    &[0, 0, 0x10, 0x20],
                    fail_data_raw.as_slice(),
                ]
                .concat()
                .to_vec(),
            ),
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_step_failure() {
        let (mut b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let req_id = tok.req_id;
        let fail_code = EcssEnumU32::new(0x1020);
        let fail_data: f32 = -22.3232;
        let mut fail_data_raw = [0; 4];
        fail_data_raw.copy_from_slice(fail_data.to_be_bytes().as_slice());
        let fail_step = EcssEnumU8::new(1);
        let fail_params = FailParamsWithStep::new(
            &EMPTY_STAMP,
            &fail_step,
            &fail_code,
            Some(fail_data_raw.as_slice()),
        );

        let accepted_token =
            b.vr.acceptance_success(tok, &mut sender, &EMPTY_STAMP)
                .expect("Sending acceptance success failed");
        let started_token =
            b.vr.start_success(accepted_token, &mut sender, &[0, 1, 0, 1, 0, 1, 0])
                .expect("Sending start success failed");
        let mut empty =
            b.vr.step_success(
                &started_token,
                &mut sender,
                &EMPTY_STAMP,
                EcssEnumU8::new(0),
            )
            .expect("Sending completion success failed");
        assert_eq!(empty, ());
        empty =
            b.vr.step_failure(started_token, &mut sender, fail_params)
                .expect("Step failure failed");
        assert_eq!(empty, ());
        check_step_failure(&mut sender, req_id, fail_data_raw);
    }

    #[test]
    fn test_steps_failure_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        let req_id = tok.req_id;
        let fail_code = EcssEnumU32::new(0x1020);
        let fail_data: f32 = -22.3232;
        let mut fail_data_raw = [0; 4];
        fail_data_raw.copy_from_slice(fail_data.to_be_bytes().as_slice());
        let fail_step = EcssEnumU8::new(1);
        let fail_params = FailParamsWithStep::new(
            &EMPTY_STAMP,
            &fail_step,
            &fail_code,
            Some(fail_data_raw.as_slice()),
        );

        let accepted_token = b
            .helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let started_token = b
            .helper
            .start_success(accepted_token, &[0, 1, 0, 1, 0, 1, 0])
            .expect("Sending start success failed");
        let mut empty = b
            .helper
            .step_success(&started_token, &EMPTY_STAMP, EcssEnumU8::new(0))
            .expect("Sending completion success failed");
        assert_eq!(empty, ());
        empty = b
            .helper
            .step_failure(started_token, fail_params)
            .expect("Step failure failed");
        assert_eq!(empty, ());
        let sender: &mut TestSender = b.helper.sender.downcast_mut().unwrap();
        check_step_failure(sender, req_id, fail_data_raw);
    }

    fn completion_fail_check(sender: &mut TestSender, req_id: RequestId) {
        assert_eq!(sender.service_queue.len(), 3);

        let mut cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 1,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: None,
            req_id,
        };
        let mut info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 3,
                apid: TEST_APID,
                msg_counter: 1,
                dest_id: 0,
                time_stamp: [0, 1, 0, 1, 0, 1, 0],
            },
            additional_data: None,
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 8,
                apid: TEST_APID,
                msg_counter: 2,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([0, 0, 0x10, 0x20].to_vec()),
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_completion_failure() {
        let (mut b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let req_id = tok.req_id;
        let fail_code = EcssEnumU32::new(0x1020);
        let fail_params = FailParams::new(&EMPTY_STAMP, &fail_code, None);

        let accepted_token =
            b.vr.acceptance_success(tok, &mut sender, &EMPTY_STAMP)
                .expect("Sending acceptance success failed");
        let started_token =
            b.vr.start_success(accepted_token, &mut sender, &[0, 1, 0, 1, 0, 1, 0])
                .expect("Sending start success failed");
        let empty =
            b.vr.completion_failure(started_token, &mut sender, fail_params)
                .expect("Completion failure");
        assert_eq!(empty, ());
        completion_fail_check(&mut sender, req_id);
    }

    #[test]
    fn test_completion_failure_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        let req_id = tok.req_id;
        let fail_code = EcssEnumU32::new(0x1020);
        let fail_params = FailParams::new(&EMPTY_STAMP, &fail_code, None);

        let accepted_token = b
            .helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let started_token = b
            .helper
            .start_success(accepted_token, &[0, 1, 0, 1, 0, 1, 0])
            .expect("Sending start success failed");
        let empty = b
            .helper
            .completion_failure(started_token, fail_params)
            .expect("Completion failure");
        assert_eq!(empty, ());
        let sender: &mut TestSender = b.helper.sender.downcast_mut().unwrap();
        completion_fail_check(sender, req_id);
    }

    fn completion_success_check(sender: &mut TestSender, req_id: RequestId) {
        assert_eq!(sender.service_queue.len(), 3);
        let cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 1,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: None,
            req_id,
        };
        let mut info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);

        let cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 3,
                apid: TEST_APID,
                msg_counter: 1,
                dest_id: 0,
                time_stamp: [0, 1, 0, 1, 0, 1, 0],
            },
            additional_data: None,
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
        let cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 7,
                apid: TEST_APID,
                msg_counter: 2,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: None,
            req_id,
        };
        info = sender.service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_complete_success_sequence() {
        let (mut b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let accepted_token =
            b.vr.acceptance_success(tok, &mut sender, &EMPTY_STAMP)
                .expect("Sending acceptance success failed");
        let started_token =
            b.vr.start_success(accepted_token, &mut sender, &[0, 1, 0, 1, 0, 1, 0])
                .expect("Sending start success failed");
        let empty =
            b.vr.completion_success(started_token, &mut sender, &EMPTY_STAMP)
                .expect("Sending completion success failed");
        assert_eq!(empty, ());
        completion_success_check(&mut sender, tok.req_id);
    }

    #[test]
    fn test_complete_success_sequence_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        let accepted_token = b
            .helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let started_token = b
            .helper
            .start_success(accepted_token, &[0, 1, 0, 1, 0, 1, 0])
            .expect("Sending start success failed");
        let empty = b
            .helper
            .completion_success(started_token, &EMPTY_STAMP)
            .expect("Sending completion success failed");
        assert_eq!(empty, ());
        let sender: &mut TestSender = b.helper.sender.downcast_mut().unwrap();
        completion_success_check(sender, tok.req_id);
    }
}
