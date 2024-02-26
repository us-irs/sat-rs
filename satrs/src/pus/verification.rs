//! # PUS Service 1 Verification Module
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
//! use satrs::pool::{PoolProviderWithGuards, StaticMemoryPool, StaticPoolConfig};
//! use satrs::pus::verification::{
//!     VerificationReportingProvider, VerificationReporterCfg, VerificationReporterWithSender
//! };
//! use satrs::seq_count::SeqCountProviderSimple;
//! use satrs::pus::TmInSharedPoolSenderWithMpsc;
//! use satrs::tmtc::tm_helper::SharedTmPool;
//! use spacepackets::ecss::PusPacket;
//! use spacepackets::SpHeader;
//! use spacepackets::ecss::tc::{PusTcCreator, PusTcSecondaryHeader};
//! use spacepackets::ecss::tm::PusTmReader;
//!
//! const EMPTY_STAMP: [u8; 7] = [0; 7];
//! const TEST_APID: u16 = 0x02;
//!
//! let pool_cfg = StaticPoolConfig::new(vec![(10, 32), (10, 64), (10, 128), (10, 1024)], false);
//! let tm_pool = StaticMemoryPool::new(pool_cfg.clone());
//! let shared_tm_store = SharedTmPool::new(tm_pool);
//! let tm_store = shared_tm_store.clone_backing_pool();
//! let (verif_tx, verif_rx) = mpsc::channel();
//! let sender = TmInSharedPoolSenderWithMpsc::new(0, "Test Sender", shared_tm_store, verif_tx);
//! let cfg = VerificationReporterCfg::new(TEST_APID, 1, 2, 8).unwrap();
//! let mut  reporter = VerificationReporterWithSender::new(&cfg , sender);
//!
//! let mut sph = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
//! let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
//! let pus_tc_0 = PusTcCreator::new_no_app_data(&mut sph, tc_header, true);
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
//!         let mut rg = tm_store.write().expect("Error locking shared pool");
//!         let store_guard = rg.read_with_guard(addr);
//!         tm_len = store_guard.read(&mut tm_buf).expect("Error reading TM slice");
//!     }
//!     let (pus_tm, _) = PusTmReader::new(&tm_buf[0..tm_len], 7)
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
use crate::pus::{source_buffer_large_enough, EcssTmSenderCore, EcssTmtcError};
use core::fmt::{Debug, Display, Formatter};
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::size_of;
#[cfg(feature = "alloc")]
use delegate::delegate;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use spacepackets::ecss::tc::IsPusTelecommand;
use spacepackets::ecss::tm::{PusTmCreator, PusTmSecondaryHeader};
use spacepackets::ecss::{EcssEnumeration, PusError, WritablePusPacket};
use spacepackets::{CcsdsPacket, PacketId, PacketSequenceCtrl};
use spacepackets::{SpHeader, MAX_APID};

pub use crate::seq_count::SeqCountProviderSimple;
pub use spacepackets::ecss::verification::*;

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "doc_cfg", doc(cfg(feature = "alloc")))]
pub use alloc_mod::*;

#[cfg(feature = "std")]
#[cfg_attr(feature = "doc_cfg", doc(cfg(feature = "std")))]
pub use std_mod::*;

/// This is a request identifier as specified in 5.4.11.2 c. of the PUS standard.
///
/// This field equivalent to the first two bytes of the CCSDS space packet header.
/// This version of the request ID is supplied in the verification reports and does not contain
/// the source ID.
#[derive(Debug, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
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
    pub fn new(tc: &(impl CcsdsPacket + IsPusTelecommand)) -> Self {
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
pub struct VerificationOrSendErrorWithToken<T>(pub EcssTmtcError, pub VerificationToken<T>);

#[derive(Debug, Clone)]
pub struct VerificationErrorWithToken<T>(pub EcssTmtcError, pub VerificationToken<T>);

impl<T> From<VerificationErrorWithToken<T>> for VerificationOrSendErrorWithToken<T> {
    fn from(value: VerificationErrorWithToken<T>) -> Self {
        VerificationOrSendErrorWithToken(value.0, value.1)
    }
}
/// Support token to allow type-state programming. This prevents calling the verification
/// steps in an invalid order.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct VerificationToken<STATE> {
    state: PhantomData<STATE>,
    req_id: RequestId,
}

pub trait WasAtLeastAccepted {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TcStateNone;
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TcStateAccepted;
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TcStateStarted;
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TcStateCompleted;

impl WasAtLeastAccepted for TcStateAccepted {}
impl WasAtLeastAccepted for TcStateStarted {}
impl WasAtLeastAccepted for TcStateCompleted {}

/// Token wrapper to model all possible verification tokens. These tokens are used to
/// enforce the correct order for the verification steps when doing verification reporting.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TcStateToken {
    None(VerificationToken<TcStateNone>),
    Accepted(VerificationToken<TcStateAccepted>),
    Started(VerificationToken<TcStateStarted>),
    Completed(VerificationToken<TcStateCompleted>),
}

impl From<VerificationToken<TcStateNone>> for TcStateToken {
    fn from(t: VerificationToken<TcStateNone>) -> Self {
        TcStateToken::None(t)
    }
}

impl TryFrom<TcStateToken> for VerificationToken<TcStateAccepted> {
    type Error = ();

    fn try_from(value: TcStateToken) -> Result<Self, Self::Error> {
        if let TcStateToken::Accepted(token) = value {
            Ok(token)
        } else {
            Err(())
        }
    }
}

impl TryFrom<TcStateToken> for VerificationToken<TcStateStarted> {
    type Error = ();

    fn try_from(value: TcStateToken) -> Result<Self, Self::Error> {
        if let TcStateToken::Started(token) = value {
            Ok(token)
        } else {
            Err(())
        }
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

impl From<VerificationToken<TcStateCompleted>> for TcStateToken {
    fn from(t: VerificationToken<TcStateCompleted>) -> Self {
        TcStateToken::Completed(t)
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
pub struct FailParams<'stamp, 'fargs> {
    time_stamp: &'stamp [u8],
    failure_code: &'fargs dyn EcssEnumeration,
    failure_data: &'fargs [u8],
}

impl<'stamp, 'fargs> FailParams<'stamp, 'fargs> {
    pub fn new(
        time_stamp: &'stamp [u8],
        failure_code: &'fargs impl EcssEnumeration,
        failure_data: &'fargs [u8],
    ) -> Self {
        Self {
            time_stamp,
            failure_code,
            failure_data,
        }
    }

    pub fn new_no_fail_data(
        time_stamp: &'stamp [u8],
        failure_code: &'fargs impl EcssEnumeration,
    ) -> Self {
        Self::new(time_stamp, failure_code, &[])
    }
}

/// Composite helper struct to pass step failure parameters to the [VerificationReporter]
pub struct FailParamsWithStep<'stamp, 'fargs> {
    bp: FailParams<'stamp, 'fargs>,
    step: &'fargs dyn EcssEnumeration,
}

impl<'stamp, 'fargs> FailParamsWithStep<'stamp, 'fargs> {
    pub fn new(
        time_stamp: &'stamp [u8],
        step: &'fargs impl EcssEnumeration,
        failure_code: &'fargs impl EcssEnumeration,
        failure_data: &'fargs [u8],
    ) -> Self {
        Self {
            bp: FailParams::new(time_stamp, failure_code, failure_data),
            step,
        }
    }
}

#[derive(Clone)]
pub struct VerificationReporterCore {
    pub dest_id: u16,
    apid: u16,
}

pub enum VerifSuccess {}
pub enum VerifFailure {}

/// Abstraction for a sendable PUS TM. The user is expected to send the TM packet to a TM sink.
///
/// This struct generally mutably borrows the source data buffer.
pub struct VerificationSendable<'src_data, State, SuccessOrFailure> {
    token: Option<VerificationToken<State>>,
    pus_tm: Option<PusTmCreator<'src_data>>,
    phantom: PhantomData<SuccessOrFailure>,
}

impl<'src_data, State, SuccessOrFailure> VerificationSendable<'src_data, State, SuccessOrFailure> {
    pub(crate) fn new(pus_tm: PusTmCreator<'src_data>, token: VerificationToken<State>) -> Self {
        Self {
            token: Some(token),
            pus_tm: Some(pus_tm),
            phantom: PhantomData,
        }
    }
    pub(crate) fn new_no_token(pus_tm: PusTmCreator<'src_data>) -> Self {
        Self {
            token: None,
            pus_tm: Some(pus_tm),
            phantom: PhantomData,
        }
    }

    pub fn len_packed(&self) -> usize {
        self.pus_tm.as_ref().unwrap().len_written()
    }

    pub fn pus_tm(&self) -> &PusTmCreator<'src_data> {
        self.pus_tm.as_ref().unwrap()
    }

    pub fn pus_tm_mut(&mut self) -> &mut PusTmCreator<'src_data> {
        self.pus_tm.as_mut().unwrap()
    }
}

impl<'src_data, State> VerificationSendable<'src_data, State, VerifFailure> {
    pub fn send_success_verif_failure(self) {}
}

impl<'src_data, State> VerificationSendable<'src_data, State, VerifFailure> {
    pub fn send_failure(self) -> (PusTmCreator<'src_data>, VerificationToken<State>) {
        (self.pus_tm.unwrap(), self.token.unwrap())
    }
}

impl<'src_data> VerificationSendable<'src_data, TcStateNone, VerifSuccess> {
    pub fn send_success_acceptance_success(self) -> VerificationToken<TcStateAccepted> {
        VerificationToken {
            state: PhantomData,
            req_id: self.token.unwrap().req_id(),
        }
    }
}

impl<'src_data> VerificationSendable<'src_data, TcStateAccepted, VerifSuccess> {
    pub fn send_success_start_success(self) -> VerificationToken<TcStateStarted> {
        VerificationToken {
            state: PhantomData,
            req_id: self.token.unwrap().req_id(),
        }
    }
}

impl<'src_data, TcState: WasAtLeastAccepted + Copy>
    VerificationSendable<'src_data, TcState, VerifSuccess>
{
    pub fn send_success_step_or_completion_success(self) {}
}

pub trait VerificationReportingProvider {
    fn add_tc(
        &mut self,
        pus_tc: &(impl CcsdsPacket + IsPusTelecommand),
    ) -> VerificationToken<TcStateNone> {
        self.add_tc_with_req_id(RequestId::new(pus_tc))
    }

    fn add_tc_with_req_id(&mut self, req_id: RequestId) -> VerificationToken<TcStateNone>;

    fn acceptance_success(
        &self,
        token: VerificationToken<TcStateNone>,
        time_stamp: &[u8],
    ) -> Result<VerificationToken<TcStateAccepted>, VerificationOrSendErrorWithToken<TcStateNone>>;

    fn acceptance_failure(
        &self,
        token: VerificationToken<TcStateNone>,
        params: FailParams,
    ) -> Result<(), VerificationOrSendErrorWithToken<TcStateNone>>;

    fn start_success(
        &self,
        token: VerificationToken<TcStateAccepted>,
        time_stamp: &[u8],
    ) -> Result<VerificationToken<TcStateStarted>, VerificationOrSendErrorWithToken<TcStateAccepted>>;

    fn start_failure(
        &self,
        token: VerificationToken<TcStateAccepted>,
        params: FailParams,
    ) -> Result<(), VerificationOrSendErrorWithToken<TcStateAccepted>>;

    fn step_success(
        &self,
        token: &VerificationToken<TcStateStarted>,
        time_stamp: &[u8],
        step: impl EcssEnumeration,
    ) -> Result<(), EcssTmtcError>;

    fn step_failure(
        &self,
        token: VerificationToken<TcStateStarted>,
        params: FailParamsWithStep,
    ) -> Result<(), VerificationOrSendErrorWithToken<TcStateStarted>>;

    fn completion_success<TcState: WasAtLeastAccepted + Copy>(
        &self,
        token: VerificationToken<TcState>,
        time_stamp: &[u8],
    ) -> Result<(), VerificationOrSendErrorWithToken<TcState>>;

    fn completion_failure<TcState: WasAtLeastAccepted + Copy>(
        &self,
        token: VerificationToken<TcState>,
        params: FailParams,
    ) -> Result<(), VerificationOrSendErrorWithToken<TcState>>;
}

/// Primary verification handler. It provides an API to send PUS 1 verification telemetry packets
/// and verify the various steps of telecommand handling as specified in the PUS standard.
///
/// This is the core component which can be used without [`alloc`] support. Please note that
/// the buffer passed to the API exposes by this struct will be used to serialize the source data.
/// This buffer may not be re-used to serialize the whole telemetry because that would overwrite
/// the source data itself.
impl VerificationReporterCore {
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
    pub fn add_tc(
        &mut self,
        pus_tc: &(impl CcsdsPacket + IsPusTelecommand),
    ) -> VerificationToken<TcStateNone> {
        self.add_tc_with_req_id(RequestId::new(pus_tc))
    }

    /// Same as [Self::add_tc] but pass a request ID instead of the direct telecommand.
    /// This can be useful if the executing thread does not have full access to the telecommand.
    pub fn add_tc_with_req_id(&mut self, req_id: RequestId) -> VerificationToken<TcStateNone> {
        VerificationToken::<TcStateNone>::new(req_id)
    }

    fn sendable_success_no_step<'src_data, State: Copy>(
        &self,
        src_data_buf: &'src_data mut [u8],
        subservice: u8,
        token: VerificationToken<State>,
        seq_count: u16,
        msg_count: u16,
        time_stamp: &'src_data [u8],
    ) -> Result<
        VerificationSendable<'src_data, State, VerifSuccess>,
        VerificationErrorWithToken<State>,
    > {
        Ok(VerificationSendable::new(
            self.create_pus_verif_success_tm(
                src_data_buf,
                subservice,
                seq_count,
                msg_count,
                &token.req_id,
                time_stamp,
                None::<&dyn EcssEnumeration>,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?,
            token,
        ))
    }

    // Internal helper function, too many arguments is acceptable for this case.
    #[allow(clippy::too_many_arguments)]
    fn sendable_failure_no_step<'src_data, State: Copy>(
        &self,
        src_data_buf: &'src_data mut [u8],
        subservice: u8,
        token: VerificationToken<State>,
        seq_count: u16,
        msg_count: u16,
        step: Option<&(impl EcssEnumeration + ?Sized)>,
        params: &FailParams<'src_data, '_>,
    ) -> Result<
        VerificationSendable<'src_data, State, VerifFailure>,
        VerificationErrorWithToken<State>,
    > {
        Ok(VerificationSendable::new(
            self.create_pus_verif_fail_tm(
                src_data_buf,
                subservice,
                seq_count,
                msg_count,
                &token.req_id,
                step,
                params,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?,
            token,
        ))
    }

    /// Package a PUS TM\[1, 1\] packet, see 8.1.2.1 of the PUS standard.
    pub fn acceptance_success<'src_data>(
        &self,
        src_data_buf: &'src_data mut [u8],
        token: VerificationToken<TcStateNone>,
        seq_count: u16,
        msg_count: u16,
        time_stamp: &'src_data [u8],
    ) -> Result<
        VerificationSendable<'src_data, TcStateNone, VerifSuccess>,
        VerificationErrorWithToken<TcStateNone>,
    > {
        self.sendable_success_no_step(
            src_data_buf,
            Subservice::TmAcceptanceSuccess.into(),
            token,
            seq_count,
            msg_count,
            time_stamp,
        )
    }

    pub fn send_acceptance_success(
        &self,
        mut sendable: VerificationSendable<'_, TcStateNone, VerifSuccess>,
        sender: &(impl EcssTmSenderCore + ?Sized),
    ) -> Result<VerificationToken<TcStateAccepted>, VerificationOrSendErrorWithToken<TcStateNone>>
    {
        sender
            .send_tm(sendable.pus_tm.take().unwrap().into())
            .map_err(|e| VerificationOrSendErrorWithToken(e, sendable.token.unwrap()))?;
        Ok(sendable.send_success_acceptance_success())
    }

    pub fn send_acceptance_failure(
        &self,
        mut sendable: VerificationSendable<'_, TcStateNone, VerifFailure>,
        sender: &(impl EcssTmSenderCore + ?Sized),
    ) -> Result<(), VerificationOrSendErrorWithToken<TcStateNone>> {
        sender
            .send_tm(sendable.pus_tm.take().unwrap().into())
            .map_err(|e| VerificationOrSendErrorWithToken(e, sendable.token.unwrap()))?;
        sendable.send_success_verif_failure();
        Ok(())
    }

    /// Package a PUS TM\[1, 2\] packet, see 8.1.2.2 of the PUS standard.
    pub fn acceptance_failure<'src_data>(
        &self,
        src_data_buf: &'src_data mut [u8],
        token: VerificationToken<TcStateNone>,
        seq_count: u16,
        msg_count: u16,
        params: FailParams<'src_data, '_>,
    ) -> Result<
        VerificationSendable<'src_data, TcStateNone, VerifFailure>,
        VerificationErrorWithToken<TcStateNone>,
    > {
        self.sendable_failure_no_step(
            src_data_buf,
            Subservice::TmAcceptanceFailure.into(),
            token,
            seq_count,
            msg_count,
            None::<&dyn EcssEnumeration>,
            &params,
        )
    }

    /// Package and send a PUS TM\[1, 3\] packet, see 8.1.2.3 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::acceptance_success].
    pub fn start_success<'src_data>(
        &self,
        src_data_buf: &'src_data mut [u8],
        token: VerificationToken<TcStateAccepted>,
        seq_count: u16,
        msg_count: u16,
        time_stamp: &'src_data [u8],
    ) -> Result<
        VerificationSendable<'src_data, TcStateAccepted, VerifSuccess>,
        VerificationErrorWithToken<TcStateAccepted>,
    > {
        self.sendable_success_no_step(
            src_data_buf,
            Subservice::TmStartSuccess.into(),
            token,
            seq_count,
            msg_count,
            time_stamp,
        )
    }

    pub fn send_start_success(
        &self,
        mut sendable: VerificationSendable<'_, TcStateAccepted, VerifSuccess>,
        sender: &(impl EcssTmSenderCore + ?Sized),
    ) -> Result<VerificationToken<TcStateStarted>, VerificationOrSendErrorWithToken<TcStateAccepted>>
    {
        sender
            .send_tm(sendable.pus_tm.take().unwrap().into())
            .map_err(|e| VerificationOrSendErrorWithToken(e, sendable.token.unwrap()))?;
        Ok(sendable.send_success_start_success())
    }

    /// Package and send a PUS TM\[1, 4\] packet, see 8.1.2.4 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::acceptance_success]. It consumes
    /// the token because verification handling is done.
    pub fn start_failure<'src_data>(
        &self,
        src_data_buf: &'src_data mut [u8],
        token: VerificationToken<TcStateAccepted>,
        seq_count: u16,
        msg_count: u16,
        params: FailParams<'src_data, '_>,
    ) -> Result<
        VerificationSendable<'src_data, TcStateAccepted, VerifFailure>,
        VerificationErrorWithToken<TcStateAccepted>,
    > {
        self.sendable_failure_no_step(
            src_data_buf,
            Subservice::TmStartFailure.into(),
            token,
            seq_count,
            msg_count,
            None::<&dyn EcssEnumeration>,
            &params,
        )
    }

    pub fn send_start_failure(
        &self,
        mut sendable: VerificationSendable<'_, TcStateAccepted, VerifFailure>,
        sender: &(impl EcssTmSenderCore + ?Sized),
    ) -> Result<(), VerificationOrSendErrorWithToken<TcStateAccepted>> {
        sender
            .send_tm(sendable.pus_tm.take().unwrap().into())
            .map_err(|e| VerificationOrSendErrorWithToken(e, sendable.token.unwrap()))?;
        sendable.send_success_verif_failure();
        Ok(())
    }

    /// Package and send a PUS TM\[1, 5\] packet, see 8.1.2.5 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::start_success].
    pub fn step_success<'src_data>(
        &self,
        src_data_buf: &'src_data mut [u8],
        token: &VerificationToken<TcStateStarted>,
        seq_count: u16,
        msg_count: u16,
        time_stamp: &'src_data [u8],
        step: impl EcssEnumeration,
    ) -> Result<VerificationSendable<'src_data, TcStateStarted, VerifSuccess>, EcssTmtcError> {
        Ok(VerificationSendable::new_no_token(
            self.create_pus_verif_success_tm(
                src_data_buf,
                Subservice::TmStepSuccess.into(),
                seq_count,
                msg_count,
                &token.req_id,
                time_stamp,
                Some(&step),
            )?,
        ))
    }

    /// Package and send a PUS TM\[1, 6\] packet, see 8.1.2.6 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
    /// token because verification handling is done.
    pub fn step_failure<'src_data>(
        &self,
        src_data_buf: &'src_data mut [u8],
        token: VerificationToken<TcStateStarted>,
        seq_count: u16,
        msg_count: u16,
        params: FailParamsWithStep<'src_data, '_>,
    ) -> Result<
        VerificationSendable<'src_data, TcStateStarted, VerifFailure>,
        VerificationErrorWithToken<TcStateStarted>,
    > {
        Ok(VerificationSendable::new(
            self.create_pus_verif_fail_tm(
                src_data_buf,
                Subservice::TmStepFailure.into(),
                seq_count,
                msg_count,
                &token.req_id,
                Some(params.step),
                &params.bp,
            )
            .map_err(|e| VerificationErrorWithToken(e, token))?,
            token,
        ))
    }

    /// Package and send a PUS TM\[1, 7\] packet, see 8.1.2.7 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
    /// token because verification handling is done.
    pub fn completion_success<'src_data, TcState: WasAtLeastAccepted + Copy>(
        &self,
        src_data_buf: &'src_data mut [u8],
        token: VerificationToken<TcState>,
        seq_counter: u16,
        msg_counter: u16,
        time_stamp: &'src_data [u8],
    ) -> Result<
        VerificationSendable<'src_data, TcState, VerifSuccess>,
        VerificationErrorWithToken<TcState>,
    > {
        self.sendable_success_no_step(
            src_data_buf,
            Subservice::TmCompletionSuccess.into(),
            token,
            seq_counter,
            msg_counter,
            time_stamp,
        )
    }

    /// Package and send a PUS TM\[1, 8\] packet, see 8.1.2.8 of the PUS standard.
    ///
    /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
    /// token because verification handling is done.
    pub fn completion_failure<'src_data, TcState: WasAtLeastAccepted + Copy>(
        &self,
        src_data_buf: &'src_data mut [u8],
        token: VerificationToken<TcState>,
        seq_count: u16,
        msg_count: u16,
        params: FailParams<'src_data, '_>,
    ) -> Result<
        VerificationSendable<'src_data, TcState, VerifFailure>,
        VerificationErrorWithToken<TcState>,
    > {
        self.sendable_failure_no_step(
            src_data_buf,
            Subservice::TmCompletionFailure.into(),
            token,
            seq_count,
            msg_count,
            None::<&dyn EcssEnumeration>,
            &params,
        )
    }

    pub fn send_step_or_completion_success<TcState: WasAtLeastAccepted + Copy>(
        &self,
        mut sendable: VerificationSendable<'_, TcState, VerifSuccess>,
        sender: &(impl EcssTmSenderCore + ?Sized),
    ) -> Result<(), VerificationOrSendErrorWithToken<TcState>> {
        sender
            .send_tm(sendable.pus_tm.take().unwrap().into())
            .map_err(|e| VerificationOrSendErrorWithToken(e, sendable.token.unwrap()))?;
        sendable.send_success_step_or_completion_success();
        Ok(())
    }

    pub fn send_step_or_completion_failure<TcState: WasAtLeastAccepted + Copy>(
        &self,
        mut sendable: VerificationSendable<'_, TcState, VerifFailure>,
        sender: &(impl EcssTmSenderCore + ?Sized),
    ) -> Result<(), VerificationOrSendErrorWithToken<TcState>> {
        sender
            .send_tm(sendable.pus_tm.take().unwrap().into())
            .map_err(|e| VerificationOrSendErrorWithToken(e, sendable.token.unwrap()))?;
        sendable.send_success_verif_failure();
        Ok(())
    }

    // Internal helper function, too many arguments is acceptable for this case.
    #[allow(clippy::too_many_arguments)]
    fn create_pus_verif_success_tm<'src_data>(
        &self,
        src_data_buf: &'src_data mut [u8],
        subservice: u8,
        seq_count: u16,
        msg_counter: u16,
        req_id: &RequestId,
        time_stamp: &'src_data [u8],
        step: Option<&(impl EcssEnumeration + ?Sized)>,
    ) -> Result<PusTmCreator<'src_data>, EcssTmtcError> {
        let mut source_data_len = size_of::<u32>();
        if let Some(step) = step {
            source_data_len += step.size();
        }
        source_buffer_large_enough(src_data_buf.len(), source_data_len)?;
        let mut idx = 0;
        req_id.to_bytes(&mut src_data_buf[0..RequestId::SIZE_AS_BYTES]);
        idx += RequestId::SIZE_AS_BYTES;
        if let Some(step) = step {
            // Size check was done beforehand
            step.write_to_be_bytes(&mut src_data_buf[idx..idx + step.size()])
                .unwrap();
        }
        let mut sp_header = SpHeader::tm_unseg(self.apid(), seq_count, 0).unwrap();
        Ok(self.create_pus_verif_tm_base(
            src_data_buf,
            subservice,
            msg_counter,
            &mut sp_header,
            time_stamp,
            source_data_len,
        ))
    }

    // Internal helper function, too many arguments is acceptable for this case.
    #[allow(clippy::too_many_arguments)]
    fn create_pus_verif_fail_tm<'src_data>(
        &self,
        src_data_buf: &'src_data mut [u8],
        subservice: u8,
        seq_count: u16,
        msg_counter: u16,
        req_id: &RequestId,
        step: Option<&(impl EcssEnumeration + ?Sized)>,
        params: &FailParams<'src_data, '_>,
    ) -> Result<PusTmCreator<'src_data>, EcssTmtcError> {
        let mut idx = 0;
        let mut source_data_len = RequestId::SIZE_AS_BYTES + params.failure_code.size();
        if let Some(step) = step {
            source_data_len += step.size();
        }
        source_data_len += params.failure_data.len();
        source_buffer_large_enough(src_data_buf.len(), source_data_len)?;
        req_id.to_bytes(&mut src_data_buf[0..RequestId::SIZE_AS_BYTES]);
        idx += RequestId::SIZE_AS_BYTES;
        if let Some(step) = step {
            // Size check done beforehand
            step.write_to_be_bytes(&mut src_data_buf[idx..idx + step.size()])
                .unwrap();
            idx += step.size();
        }
        params
            .failure_code
            .write_to_be_bytes(&mut src_data_buf[idx..idx + params.failure_code.size()])
            .map_err(PusError::ByteConversion)?;
        idx += params.failure_code.size();
        src_data_buf[idx..idx + params.failure_data.len()].copy_from_slice(params.failure_data);
        let mut sp_header = SpHeader::tm_unseg(self.apid(), seq_count, 0).unwrap();
        Ok(self.create_pus_verif_tm_base(
            src_data_buf,
            subservice,
            msg_counter,
            &mut sp_header,
            params.time_stamp,
            source_data_len,
        ))
    }

    fn create_pus_verif_tm_base<'src_data>(
        &self,
        src_data_buf: &'src_data mut [u8],
        subservice: u8,
        msg_counter: u16,
        sp_header: &mut SpHeader,
        time_stamp: &'src_data [u8],
        source_data_len: usize,
    ) -> PusTmCreator<'src_data> {
        let tm_sec_header =
            PusTmSecondaryHeader::new(1, subservice, msg_counter, self.dest_id, Some(time_stamp));
        PusTmCreator::new(
            sp_header,
            tm_sec_header,
            &src_data_buf[0..source_data_len],
            true,
        )
    }
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use super::*;
    use crate::{
        pus::{TmAsVecSenderWithId, TmInSharedPoolSenderWithId},
        seq_count::SequenceCountProvider,
    };
    use core::cell::RefCell;

    #[derive(Clone)]
    pub struct VerificationReporterCfg {
        apid: u16,
        pub step_field_width: usize,
        pub fail_code_field_width: usize,
        pub max_fail_data_len: usize,
    }

    impl VerificationReporterCfg {
        pub fn new(
            apid: u16,
            step_field_width: usize,
            fail_code_field_width: usize,
            max_fail_data_len: usize,
        ) -> Option<Self> {
            if apid > MAX_APID {
                return None;
            }
            Some(Self {
                apid,
                step_field_width,
                fail_code_field_width,
                max_fail_data_len,
            })
        }
    }

    /// Primary verification handler. It provides an API to send PUS 1 verification telemetry packets
    /// and verify the various steps of telecommand handling as specified in the PUS standard.
    /// It is assumed that the sequence counter and message counters are updated in a central
    /// TM funnel. This helper will always set those fields to 0.
    #[derive(Clone)]
    pub struct VerificationReporter {
        source_data_buf: RefCell<alloc::vec::Vec<u8>>,
        pub seq_count_provider: Option<alloc::boxed::Box<dyn SequenceCountProvider<u16> + Send>>,
        pub msg_count_provider: Option<alloc::boxed::Box<dyn SequenceCountProvider<u16> + Send>>,
        pub reporter: VerificationReporterCore,
    }

    impl VerificationReporter {
        pub fn new(cfg: &VerificationReporterCfg) -> Self {
            let reporter = VerificationReporterCore::new(cfg.apid).unwrap();
            Self {
                source_data_buf: RefCell::new(alloc::vec![
                    0;
                    RequestId::SIZE_AS_BYTES
                        + cfg.step_field_width
                        + cfg.fail_code_field_width
                        + cfg.max_fail_data_len
                ]),
                seq_count_provider: None,
                msg_count_provider: None,
                reporter,
            }
        }

        delegate!(
            to self.reporter {
                pub fn set_apid(&mut self, apid: u16) -> bool;
                pub fn apid(&self) -> u16;
                pub fn add_tc(&mut self, pus_tc: &(impl CcsdsPacket + IsPusTelecommand)) -> VerificationToken<TcStateNone>;
                pub fn add_tc_with_req_id(&mut self, req_id: RequestId) -> VerificationToken<TcStateNone>;
                pub fn dest_id(&self) -> u16;
                pub fn set_dest_id(&mut self, dest_id: u16);
            }
        );

        pub fn allowed_source_data_len(&self) -> usize {
            self.source_data_buf.borrow().capacity()
        }

        /// Package and send a PUS TM\[1, 1\] packet, see 8.1.2.1 of the PUS standard
        pub fn acceptance_success(
            &self,
            token: VerificationToken<TcStateNone>,
            sender: &(impl EcssTmSenderCore + ?Sized),
            time_stamp: &[u8],
        ) -> Result<VerificationToken<TcStateAccepted>, VerificationOrSendErrorWithToken<TcStateNone>>
        {
            let seq_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let msg_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let mut source_data_buf = self.source_data_buf.borrow_mut();
            let sendable = self.reporter.acceptance_success(
                source_data_buf.as_mut_slice(),
                token,
                seq_count,
                msg_count,
                time_stamp,
            )?;
            self.reporter.send_acceptance_success(sendable, sender)
        }

        /// Package and send a PUS TM\[1, 2\] packet, see 8.1.2.2 of the PUS standard
        pub fn acceptance_failure(
            &self,
            token: VerificationToken<TcStateNone>,
            sender: &(impl EcssTmSenderCore + ?Sized),
            params: FailParams,
        ) -> Result<(), VerificationOrSendErrorWithToken<TcStateNone>> {
            let seq_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let msg_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let mut buf = self.source_data_buf.borrow_mut();
            let sendable = self.reporter.acceptance_failure(
                buf.as_mut_slice(),
                token,
                seq_count,
                msg_count,
                params,
            )?;
            self.reporter.send_acceptance_failure(sendable, sender)
        }

        /// Package and send a PUS TM\[1, 3\] packet, see 8.1.2.3 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::acceptance_success].
        pub fn start_success(
            &self,
            token: VerificationToken<TcStateAccepted>,
            sender: &(impl EcssTmSenderCore + ?Sized),
            time_stamp: &[u8],
        ) -> Result<
            VerificationToken<TcStateStarted>,
            VerificationOrSendErrorWithToken<TcStateAccepted>,
        > {
            let seq_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let msg_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let mut buf = self.source_data_buf.borrow_mut();
            let sendable = self.reporter.start_success(
                buf.as_mut_slice(),
                token,
                seq_count,
                msg_count,
                time_stamp,
            )?;
            self.reporter.send_start_success(sendable, sender)
        }

        /// Package and send a PUS TM\[1, 4\] packet, see 8.1.2.4 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::acceptance_success]. It consumes
        /// the token because verification handling is done.
        pub fn start_failure(
            &self,
            token: VerificationToken<TcStateAccepted>,
            sender: &(impl EcssTmSenderCore + ?Sized),
            params: FailParams,
        ) -> Result<(), VerificationOrSendErrorWithToken<TcStateAccepted>> {
            let seq_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let msg_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let mut buf = self.source_data_buf.borrow_mut();
            let sendable = self.reporter.start_failure(
                buf.as_mut_slice(),
                token,
                seq_count,
                msg_count,
                params,
            )?;
            self.reporter.send_start_failure(sendable, sender)
        }

        /// Package and send a PUS TM\[1, 5\] packet, see 8.1.2.5 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::start_success].
        pub fn step_success(
            &self,
            token: &VerificationToken<TcStateStarted>,
            sender: &(impl EcssTmSenderCore + ?Sized),
            time_stamp: &[u8],
            step: impl EcssEnumeration,
        ) -> Result<(), EcssTmtcError> {
            let seq_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let msg_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let mut buf = self.source_data_buf.borrow_mut();
            let sendable = self.reporter.step_success(
                buf.as_mut_slice(),
                token,
                seq_count,
                msg_count,
                time_stamp,
                step,
            )?;
            self.reporter
                .send_step_or_completion_success(sendable, sender)
                .map_err(|e| e.0)
        }

        /// Package and send a PUS TM\[1, 6\] packet, see 8.1.2.6 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
        /// token because verification handling is done.
        pub fn step_failure(
            &self,
            token: VerificationToken<TcStateStarted>,
            sender: &(impl EcssTmSenderCore + ?Sized),
            params: FailParamsWithStep,
        ) -> Result<(), VerificationOrSendErrorWithToken<TcStateStarted>> {
            let seq_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let msg_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let mut buf = self.source_data_buf.borrow_mut();
            let sendable = self.reporter.step_failure(
                buf.as_mut_slice(),
                token,
                seq_count,
                msg_count,
                params,
            )?;
            self.reporter
                .send_step_or_completion_failure(sendable, sender)
        }

        /// Package and send a PUS TM\[1, 7\] packet, see 8.1.2.7 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
        /// token because verification handling is done.
        pub fn completion_success<TcState: WasAtLeastAccepted + Copy>(
            &self,
            token: VerificationToken<TcState>,
            sender: &(impl EcssTmSenderCore + ?Sized),
            time_stamp: &[u8],
        ) -> Result<(), VerificationOrSendErrorWithToken<TcState>> {
            let seq_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let msg_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let mut buf = self.source_data_buf.borrow_mut();
            let sendable = self.reporter.completion_success(
                buf.as_mut_slice(),
                token,
                seq_count,
                msg_count,
                time_stamp,
            )?;
            self.reporter
                .send_step_or_completion_success(sendable, sender)
        }

        /// Package and send a PUS TM\[1, 8\] packet, see 8.1.2.8 of the PUS standard.
        ///
        /// Requires a token previously acquired by calling [Self::start_success]. It consumes the
        /// token because verification handling is done.
        pub fn completion_failure<TcState: WasAtLeastAccepted + Copy>(
            &self,
            token: VerificationToken<TcState>,
            sender: &(impl EcssTmSenderCore + ?Sized),
            params: FailParams,
        ) -> Result<(), VerificationOrSendErrorWithToken<TcState>> {
            let seq_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let msg_count = self
                .seq_count_provider
                .as_ref()
                .map_or(0, |v| v.get_and_increment());
            let mut buf = self.source_data_buf.borrow_mut();
            let sendable = self.reporter.completion_failure(
                buf.as_mut_slice(),
                token,
                seq_count,
                msg_count,
                params,
            )?;
            self.reporter
                .send_step_or_completion_failure(sendable, sender)
        }
    }

    /// Helper object which caches the sender passed as a trait object. Provides the same
    /// API as [VerificationReporter] but without the explicit sender arguments.
    #[derive(Clone)]
    pub struct VerificationReporterWithSender<Sender: EcssTmSenderCore + Clone> {
        pub reporter: VerificationReporter,
        pub sender: Sender,
    }

    impl<Sender: EcssTmSenderCore + Clone> VerificationReporterWithSender<Sender> {
        pub fn new(cfg: &VerificationReporterCfg, sender: Sender) -> Self {
            let reporter = VerificationReporter::new(cfg);
            Self::new_from_reporter(reporter, sender)
        }

        pub fn new_from_reporter(reporter: VerificationReporter, sender: Sender) -> Self {
            Self { reporter, sender }
        }

        delegate! {
            to self.reporter {
                pub fn set_apid(&mut self, apid: u16) -> bool;
                pub fn apid(&self) -> u16;
                pub fn dest_id(&self) -> u16;
                pub fn set_dest_id(&mut self, dest_id: u16);
            }
        }
    }

    impl<Sender: EcssTmSenderCore + Clone> VerificationReportingProvider
        for VerificationReporterWithSender<Sender>
    {
        delegate! {
            to self.reporter {
                fn add_tc(
                    &mut self,
                    pus_tc: &(impl CcsdsPacket + IsPusTelecommand),
                ) -> VerificationToken<TcStateNone>;
                fn add_tc_with_req_id(&mut self, req_id: RequestId) -> VerificationToken<TcStateNone>;
            }
        }

        fn acceptance_success(
            &self,
            token: VerificationToken<TcStateNone>,
            time_stamp: &[u8],
        ) -> Result<VerificationToken<TcStateAccepted>, VerificationOrSendErrorWithToken<TcStateNone>>
        {
            self.reporter
                .acceptance_success(token, &self.sender, time_stamp)
        }

        fn acceptance_failure(
            &self,
            token: VerificationToken<TcStateNone>,
            params: FailParams,
        ) -> Result<(), VerificationOrSendErrorWithToken<TcStateNone>> {
            self.reporter
                .acceptance_failure(token, &self.sender, params)
        }

        fn start_success(
            &self,
            token: VerificationToken<TcStateAccepted>,
            time_stamp: &[u8],
        ) -> Result<
            VerificationToken<TcStateStarted>,
            VerificationOrSendErrorWithToken<TcStateAccepted>,
        > {
            self.reporter.start_success(token, &self.sender, time_stamp)
        }

        fn start_failure(
            &self,
            token: VerificationToken<TcStateAccepted>,
            params: FailParams,
        ) -> Result<(), VerificationOrSendErrorWithToken<TcStateAccepted>> {
            self.reporter.start_failure(token, &self.sender, params)
        }

        fn step_success(
            &self,
            token: &VerificationToken<TcStateStarted>,
            time_stamp: &[u8],
            step: impl EcssEnumeration,
        ) -> Result<(), EcssTmtcError> {
            self.reporter
                .step_success(token, &self.sender, time_stamp, step)
        }

        fn step_failure(
            &self,
            token: VerificationToken<TcStateStarted>,
            params: FailParamsWithStep,
        ) -> Result<(), VerificationOrSendErrorWithToken<TcStateStarted>> {
            self.reporter.step_failure(token, &self.sender, params)
        }

        fn completion_success<TcState: WasAtLeastAccepted + Copy>(
            &self,
            token: VerificationToken<TcState>,
            time_stamp: &[u8],
        ) -> Result<(), VerificationOrSendErrorWithToken<TcState>> {
            self.reporter
                .completion_success(token, &self.sender, time_stamp)
        }

        fn completion_failure<TcState: WasAtLeastAccepted + Copy>(
            &self,
            token: VerificationToken<TcState>,
            params: FailParams,
        ) -> Result<(), VerificationOrSendErrorWithToken<TcState>> {
            self.reporter
                .completion_failure(token, &self.sender, params)
        }
    }

    pub type VerificationReporterWithSharedPoolSender<Sender> =
        VerificationReporterWithSender<TmInSharedPoolSenderWithId<Sender>>;
    pub type VerificationReporterWithVecSender<Sender> =
        VerificationReporterWithSender<TmAsVecSenderWithId<Sender>>;
}

#[cfg(feature = "std")]
pub mod std_mod {
    use std::sync::mpsc;

    use crate::pool::StoreAddr;

    use super::alloc_mod::{
        VerificationReporterWithSharedPoolSender, VerificationReporterWithVecSender,
    };

    pub type VerificationReporterWithSharedPoolMpscSender =
        VerificationReporterWithSharedPoolSender<mpsc::Sender<StoreAddr>>;
    pub type VerificationReporterWithSharedPoolMpscBoundedSender =
        VerificationReporterWithSharedPoolSender<mpsc::SyncSender<StoreAddr>>;
    pub type VerificationReporterWithVecMpscSender =
        VerificationReporterWithVecSender<mpsc::Sender<alloc::vec::Vec<u8>>>;
    pub type VerificationReporterWithVecMpscBoundedSender =
        VerificationReporterWithVecSender<mpsc::SyncSender<alloc::vec::Vec<u8>>>;
}

#[cfg(test)]
pub mod tests {
    use crate::pool::{PoolProviderWithGuards, StaticMemoryPool, StaticPoolConfig};
    use crate::pus::tests::CommonTmInfo;
    use crate::pus::verification::{
        EcssTmSenderCore, EcssTmtcError, FailParams, FailParamsWithStep, RequestId, TcStateNone,
        VerificationReporter, VerificationReporterCfg, VerificationReporterWithSender,
        VerificationToken,
    };
    use crate::pus::{
        EcssChannel, PusTmWrapper, TmInSharedPoolSenderWithId, TmInSharedPoolSenderWithMpsc,
    };
    use crate::tmtc::tm_helper::SharedTmPool;
    use crate::ChannelId;
    use alloc::format;
    use alloc::sync::Arc;
    use hashbrown::HashMap;
    use spacepackets::ecss::tc::{PusTcCreator, PusTcSecondaryHeader};
    use spacepackets::ecss::tm::PusTmReader;
    use spacepackets::ecss::{EcssEnumU16, EcssEnumU32, EcssEnumU8, PusError, PusPacket};
    use spacepackets::util::UnsignedEnum;
    use spacepackets::{ByteConversionError, CcsdsPacket, SpHeader};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::sync::{mpsc, Mutex};
    use std::time::Duration;
    use std::vec;
    use std::vec::Vec;

    use super::VerificationReportingProvider;

    fn is_send<T: Send>(_: &T) {}
    #[allow(dead_code)]
    fn is_sync<T: Sync>(_: &T) {}

    pub struct VerificationStatus {
        pub accepted: Option<bool>,
        pub started: Option<bool>,
        pub step: u64,
        pub step_status: Option<bool>,
        pub completed: Option<bool>,
        pub failure_data: Option<Vec<u8>>,
        pub fail_enum: Option<u64>,
    }

    pub type SharedVerificationMap = Arc<Mutex<RefCell<HashMap<RequestId, VerificationStatus>>>>;

    #[derive(Clone)]
    pub struct TestVerificationReporter {
        pub verification_map: SharedVerificationMap,
    }

    impl TestVerificationReporter {
        pub fn new(verification_map: SharedVerificationMap) -> Self {
            Self { verification_map }
        }
    }

    impl VerificationReportingProvider for TestVerificationReporter {
        fn add_tc_with_req_id(&mut self, req_id: RequestId) -> VerificationToken<TcStateNone> {
            let verif_map = self.verification_map.lock().unwrap();
            verif_map.borrow_mut().insert(
                req_id,
                VerificationStatus {
                    accepted: None,
                    started: None,
                    step: 0,
                    step_status: None,
                    completed: None,
                    failure_data: None,
                    fail_enum: None,
                },
            );
            VerificationToken {
                state: core::marker::PhantomData,
                req_id,
            }
        }

        fn acceptance_success(
            &self,
            token: VerificationToken<TcStateNone>,
            _time_stamp: &[u8],
        ) -> Result<
            VerificationToken<super::TcStateAccepted>,
            super::VerificationOrSendErrorWithToken<TcStateNone>,
        > {
            let verif_map = self.verification_map.lock().unwrap();
            match verif_map.borrow_mut().get_mut(&token.req_id) {
                Some(entry) => entry.accepted = Some(true),
                None => panic!(
                    "unexpected acceptance success for request ID {}",
                    token.req_id()
                ),
            };
            Ok(VerificationToken {
                state: core::marker::PhantomData,
                req_id: token.req_id,
            })
        }

        fn acceptance_failure(
            &self,
            token: VerificationToken<TcStateNone>,
            params: FailParams,
        ) -> Result<(), super::VerificationOrSendErrorWithToken<TcStateNone>> {
            let verif_map = self.verification_map.lock().unwrap();
            match verif_map.borrow_mut().get_mut(&token.req_id) {
                Some(entry) => {
                    entry.accepted = Some(false);
                    entry.failure_data = Some(params.failure_data.to_vec());
                    entry.fail_enum = Some(params.failure_code.value());
                }
                None => panic!(
                    "unexpected acceptance failure for request ID {}",
                    token.req_id()
                ),
            };
            Ok(())
        }

        fn start_success(
            &self,
            token: VerificationToken<super::TcStateAccepted>,
            _time_stamp: &[u8],
        ) -> Result<
            VerificationToken<super::TcStateStarted>,
            super::VerificationOrSendErrorWithToken<super::TcStateAccepted>,
        > {
            let verif_map = self.verification_map.lock().unwrap();
            match verif_map.borrow_mut().get_mut(&token.req_id) {
                Some(entry) => entry.started = Some(true),
                None => panic!("unexpected start success for request ID {}", token.req_id()),
            };
            Ok(VerificationToken {
                state: core::marker::PhantomData,
                req_id: token.req_id,
            })
        }

        fn start_failure(
            &self,
            token: VerificationToken<super::TcStateAccepted>,
            params: FailParams,
        ) -> Result<(), super::VerificationOrSendErrorWithToken<super::TcStateAccepted>> {
            let verif_map = self.verification_map.lock().unwrap();
            match verif_map.borrow_mut().get_mut(&token.req_id) {
                Some(entry) => {
                    entry.started = Some(false);
                    entry.failure_data = Some(params.failure_data.to_vec());
                    entry.fail_enum = Some(params.failure_code.value());
                }
                None => panic!("unexpected start failure for request ID {}", token.req_id()),
            };
            Ok(())
        }

        fn step_success(
            &self,
            token: &VerificationToken<super::TcStateStarted>,
            _time_stamp: &[u8],
            step: impl spacepackets::ecss::EcssEnumeration,
        ) -> Result<(), EcssTmtcError> {
            let verif_map = self.verification_map.lock().unwrap();
            match verif_map.borrow_mut().get_mut(&token.req_id) {
                Some(entry) => {
                    entry.step = step.value();
                    entry.step_status = Some(true);
                }
                None => panic!("unexpected start success for request ID {}", token.req_id()),
            };
            Ok(())
        }

        fn step_failure(
            &self,
            token: VerificationToken<super::TcStateStarted>,
            _params: FailParamsWithStep,
        ) -> Result<(), super::VerificationOrSendErrorWithToken<super::TcStateStarted>> {
            let verif_map = self.verification_map.lock().unwrap();
            match verif_map.borrow_mut().get_mut(&token.req_id) {
                Some(entry) => {
                    entry.step_status = Some(false);
                }
                None => panic!("unexpected start success for request ID {}", token.req_id()),
            };
            Ok(())
        }

        fn completion_success<TcState: super::WasAtLeastAccepted + Copy>(
            &self,
            token: VerificationToken<TcState>,
            _time_stamp: &[u8],
        ) -> Result<(), super::VerificationOrSendErrorWithToken<TcState>> {
            let verif_map = self.verification_map.lock().unwrap();
            match verif_map.borrow_mut().get_mut(&token.req_id) {
                Some(entry) => entry.completed = Some(true),
                None => panic!(
                    "unexpected acceptance success for request ID {}",
                    token.req_id()
                ),
            };
            Ok(())
        }

        fn completion_failure<TcState: super::WasAtLeastAccepted + Copy>(
            &self,
            token: VerificationToken<TcState>,
            params: FailParams,
        ) -> Result<(), super::VerificationOrSendErrorWithToken<TcState>> {
            let verif_map = self.verification_map.lock().unwrap();
            match verif_map.borrow_mut().get_mut(&token.req_id) {
                Some(entry) => {
                    entry.completed = Some(false);
                    entry.failure_data = Some(params.failure_data.to_vec());
                    entry.fail_enum = Some(params.failure_code.value());
                }
                None => panic!(
                    "unexpected acceptance success for request ID {}",
                    token.req_id()
                ),
            };
            Ok(())
        }
    }

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
        pub service_queue: RefCell<VecDeque<TmInfo>>,
    }

    impl EcssChannel for TestSender {
        fn channel_id(&self) -> ChannelId {
            0
        }
        fn name(&self) -> &'static str {
            "test_sender"
        }
    }

    impl EcssTmSenderCore for TestSender {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmWrapper::InStore(_) => {
                    panic!("TestSender: Can not deal with addresses");
                }
                PusTmWrapper::Direct(tm) => {
                    assert_eq!(PusPacket::service(&tm), 1);
                    assert!(!tm.source_data().is_empty());
                    let mut time_stamp = [0; 7];
                    time_stamp.clone_from_slice(&tm.timestamp()[0..7]);
                    let src_data = tm.source_data();
                    assert!(src_data.len() >= 4);
                    let req_id =
                        RequestId::from_bytes(&src_data[0..RequestId::SIZE_AS_BYTES]).unwrap();
                    let mut vec = None;
                    if src_data.len() > 4 {
                        let mut new_vec = Vec::new();
                        new_vec.extend_from_slice(&src_data[RequestId::SIZE_AS_BYTES..]);
                        vec = Some(new_vec);
                    }
                    self.service_queue.borrow_mut().push_back(TmInfo {
                        common: CommonTmInfo::new_from_tm(&tm),
                        req_id,
                        additional_data: vec,
                    });
                    Ok(())
                }
            }
        }
    }

    struct TestBase<'a> {
        vr: VerificationReporter,
        #[allow(dead_code)]
        tc: PusTcCreator<'a>,
    }

    impl<'a> TestBase<'a> {
        fn rep(&mut self) -> &mut VerificationReporter {
            &mut self.vr
        }
    }
    struct TestBaseWithHelper<'a, Sender: EcssTmSenderCore + Clone + 'static> {
        helper: VerificationReporterWithSender<Sender>,
        #[allow(dead_code)]
        tc: PusTcCreator<'a>,
    }

    impl<'a, Sender: EcssTmSenderCore + Clone + 'static> TestBaseWithHelper<'a, Sender> {
        fn rep(&mut self) -> &mut VerificationReporter {
            &mut self.helper.reporter
        }
    }

    fn base_reporter() -> VerificationReporter {
        let cfg = VerificationReporterCfg::new(TEST_APID, 1, 2, 8).unwrap();
        VerificationReporter::new(&cfg)
    }

    fn base_tc_init(app_data: Option<&[u8]>) -> (PusTcCreator, RequestId) {
        let mut sph = SpHeader::tc_unseg(TEST_APID, 0x34, 0).unwrap();
        let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
        let app_data = app_data.unwrap_or(&[]);
        let pus_tc = PusTcCreator::new(&mut sph, tc_header, app_data, true);
        let req_id = RequestId::new(&pus_tc);
        (pus_tc, req_id)
    }

    fn base_init(api_sel: bool) -> (TestBase<'static>, VerificationToken<TcStateNone>) {
        let mut reporter = base_reporter();
        let (tc, req_id) = base_tc_init(None);
        let init_tok = if api_sel {
            reporter.add_tc_with_req_id(req_id)
        } else {
            reporter.add_tc(&tc)
        };
        (TestBase { vr: reporter, tc }, init_tok)
    }

    fn base_with_helper_init() -> (
        TestBaseWithHelper<'static, TestSender>,
        VerificationToken<TcStateNone>,
    ) {
        let mut reporter = base_reporter();
        let (tc, _) = base_tc_init(None);
        let init_tok = reporter.add_tc(&tc);
        let sender = TestSender::default();
        let helper = VerificationReporterWithSender::new_from_reporter(reporter, sender);
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
            req_id: *req_id,
        };
        let mut service_queue = sender.service_queue.borrow_mut();
        assert_eq!(service_queue.len(), 1);
        let info = service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_mpsc_verif_send_sync() {
        let pool = StaticMemoryPool::new(StaticPoolConfig::new(vec![(8, 8)], false));
        let shared_tm_store = SharedTmPool::new(pool);
        let (tx, _) = mpsc::channel();
        let mpsc_verif_sender =
            TmInSharedPoolSenderWithMpsc::new(0, "verif_sender", shared_tm_store, tx);
        is_send(&mpsc_verif_sender);
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
        let (b, tok) = base_init(false);
        let mut sender = TestSender::default();
        b.vr.acceptance_success(tok, &sender, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        acceptance_check(&mut sender, &tok.req_id);
    }

    #[test]
    fn test_basic_acceptance_success_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        b.helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        acceptance_check(&mut b.helper.sender, &tok.req_id);
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
        let mut service_queue = sender.service_queue.borrow_mut();
        assert_eq!(service_queue.len(), 1);
        let info = service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_basic_acceptance_failure() {
        let (mut b, tok) = base_init(true);
        b.rep().reporter.dest_id = 5;
        let stamp_buf = [1, 2, 3, 4, 5, 6, 7];
        let mut sender = TestSender::default();
        let fail_code = EcssEnumU16::new(2);
        let fail_params = FailParams::new_no_fail_data(stamp_buf.as_slice(), &fail_code);
        b.vr.acceptance_failure(tok, &sender, fail_params)
            .expect("Sending acceptance success failed");
        acceptance_fail_check(&mut sender, tok.req_id, stamp_buf);
    }

    #[test]
    fn test_basic_acceptance_failure_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        b.rep().reporter.dest_id = 5;
        let stamp_buf = [1, 2, 3, 4, 5, 6, 7];
        let fail_code = EcssEnumU16::new(2);
        let fail_params = FailParams::new_no_fail_data(stamp_buf.as_slice(), &fail_code);
        b.helper
            .acceptance_failure(tok, fail_params)
            .expect("Sending acceptance success failed");
        acceptance_fail_check(&mut b.helper.sender, tok.req_id, stamp_buf);
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
        let fail_params = FailParams::new(stamp_buf.as_slice(), &fail_code, fail_data.as_slice());
        let res = b.helper.acceptance_failure(tok, fail_params);
        assert!(res.is_err());
        let err_with_token = res.unwrap_err();
        assert_eq!(err_with_token.1, tok);
        match err_with_token.0 {
            EcssTmtcError::Pus(PusError::ByteConversion(e)) => match e {
                ByteConversionError::ToSliceTooSmall { found, expected } => {
                    assert_eq!(
                        expected,
                        fail_data.len() + RequestId::SIZE_AS_BYTES + fail_code.size()
                    );
                    assert_eq!(found, b.rep().allowed_source_data_len());
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
        let (b, tok) = base_init(false);
        let sender = TestSender::default();
        let fail_code = EcssEnumU8::new(10);
        let fail_data = EcssEnumU32::new(12);
        let mut fail_data_raw = [0; 4];
        fail_data.write_to_be_bytes(&mut fail_data_raw).unwrap();
        let fail_params = FailParams::new(&EMPTY_STAMP, &fail_code, fail_data_raw.as_slice());
        b.vr.acceptance_failure(tok, &sender, fail_params)
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
        let mut service_queue = sender.service_queue.borrow_mut();
        assert_eq!(service_queue.len(), 1);
        let info = service_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    fn start_fail_check(sender: &mut TestSender, req_id: RequestId, fail_data_raw: [u8; 4]) {
        let mut srv_queue = sender.service_queue.borrow_mut();
        assert_eq!(srv_queue.len(), 2);
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
        let mut info = srv_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 4,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([&[22], fail_data_raw.as_slice()].concat().to_vec()),
            req_id,
        };
        info = srv_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_start_failure() {
        let (b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let fail_code = EcssEnumU8::new(22);
        let fail_data: i32 = -12;
        let mut fail_data_raw = [0; 4];
        fail_data_raw.copy_from_slice(fail_data.to_be_bytes().as_slice());
        let fail_params = FailParams::new(&EMPTY_STAMP, &fail_code, fail_data_raw.as_slice());

        let accepted_token =
            b.vr.acceptance_success(tok, &sender, &EMPTY_STAMP)
                .expect("Sending acceptance success failed");
        b.vr.start_failure(accepted_token, &sender, fail_params)
            .expect("Start failure failure");
        start_fail_check(&mut sender, tok.req_id, fail_data_raw);
    }

    #[test]
    fn test_start_failure_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        let fail_code = EcssEnumU8::new(22);
        let fail_data: i32 = -12;
        let mut fail_data_raw = [0; 4];
        fail_data_raw.copy_from_slice(fail_data.to_be_bytes().as_slice());
        let fail_params = FailParams::new(&EMPTY_STAMP, &fail_code, fail_data_raw.as_slice());

        let accepted_token = b
            .helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        b.helper
            .start_failure(accepted_token, fail_params)
            .expect("Start failure failure");
        start_fail_check(&mut b.helper.sender, tok.req_id, fail_data_raw);
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
        let mut srv_queue = sender.service_queue.borrow_mut();
        let mut info = srv_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 3,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: [0, 1, 0, 1, 0, 1, 0],
            },
            additional_data: None,
            req_id,
        };
        info = srv_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 5,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([0].to_vec()),
            req_id,
        };
        info = srv_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 5,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([1].to_vec()),
            req_id,
        };
        info = srv_queue.pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_steps_success() {
        let (mut b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let accepted_token = b
            .rep()
            .acceptance_success(tok, &sender, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let started_token = b
            .rep()
            .start_success(accepted_token, &sender, &[0, 1, 0, 1, 0, 1, 0])
            .expect("Sending start success failed");
        b.rep()
            .step_success(&started_token, &sender, &EMPTY_STAMP, EcssEnumU8::new(0))
            .expect("Sending step 0 success failed");
        b.vr.step_success(&started_token, &sender, &EMPTY_STAMP, EcssEnumU8::new(1))
            .expect("Sending step 1 success failed");
        assert_eq!(sender.service_queue.borrow().len(), 4);
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
        b.helper
            .step_success(&started_token, &EMPTY_STAMP, EcssEnumU8::new(0))
            .expect("Sending step 0 success failed");
        b.helper
            .step_success(&started_token, &EMPTY_STAMP, EcssEnumU8::new(1))
            .expect("Sending step 1 success failed");
        assert_eq!(b.helper.sender.service_queue.borrow().len(), 4);
        step_success_check(&mut b.helper.sender, tok.req_id);
    }

    fn check_step_failure(sender: &mut TestSender, req_id: RequestId, fail_data_raw: [u8; 4]) {
        assert_eq!(sender.service_queue.borrow().len(), 4);
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
        let mut info = sender.service_queue.borrow_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 3,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: [0, 1, 0, 1, 0, 1, 0],
            },
            additional_data: None,
            req_id,
        };
        info = sender.service_queue.borrow_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 5,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([0].to_vec()),
            req_id,
        };
        info = sender.service_queue.get_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 6,
                apid: TEST_APID,
                msg_counter: 0,
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
        info = sender.service_queue.get_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_step_failure() {
        let (b, tok) = base_init(false);
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
            fail_data_raw.as_slice(),
        );

        let accepted_token =
            b.vr.acceptance_success(tok, &sender, &EMPTY_STAMP)
                .expect("Sending acceptance success failed");
        let started_token =
            b.vr.start_success(accepted_token, &sender, &[0, 1, 0, 1, 0, 1, 0])
                .expect("Sending start success failed");
        b.vr.step_success(&started_token, &sender, &EMPTY_STAMP, EcssEnumU8::new(0))
            .expect("Sending completion success failed");
        b.vr.step_failure(started_token, &sender, fail_params)
            .expect("Step failure failed");
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
            fail_data_raw.as_slice(),
        );

        let accepted_token = b
            .helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let started_token = b
            .helper
            .start_success(accepted_token, &[0, 1, 0, 1, 0, 1, 0])
            .expect("Sending start success failed");
        b.helper
            .step_success(&started_token, &EMPTY_STAMP, EcssEnumU8::new(0))
            .expect("Sending completion success failed");
        b.helper
            .step_failure(started_token, fail_params)
            .expect("Step failure failed");
        check_step_failure(&mut b.helper.sender, req_id, fail_data_raw);
    }

    fn completion_fail_check(sender: &mut TestSender, req_id: RequestId) {
        assert_eq!(sender.service_queue.borrow().len(), 3);

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
        let mut info = sender.service_queue.get_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 3,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: [0, 1, 0, 1, 0, 1, 0],
            },
            additional_data: None,
            req_id,
        };
        info = sender.service_queue.get_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);

        cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 8,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: Some([0, 0, 0x10, 0x20].to_vec()),
            req_id,
        };
        info = sender.service_queue.get_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_completion_failure() {
        let (b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let req_id = tok.req_id;
        let fail_code = EcssEnumU32::new(0x1020);
        let fail_params = FailParams::new_no_fail_data(&EMPTY_STAMP, &fail_code);

        let accepted_token =
            b.vr.acceptance_success(tok, &sender, &EMPTY_STAMP)
                .expect("Sending acceptance success failed");
        let started_token =
            b.vr.start_success(accepted_token, &sender, &[0, 1, 0, 1, 0, 1, 0])
                .expect("Sending start success failed");
        b.vr.completion_failure(started_token, &sender, fail_params)
            .expect("Completion failure");
        completion_fail_check(&mut sender, req_id);
    }

    #[test]
    fn test_completion_failure_with_helper() {
        let (mut b, tok) = base_with_helper_init();
        let req_id = tok.req_id;
        let fail_code = EcssEnumU32::new(0x1020);
        let fail_params = FailParams::new_no_fail_data(&EMPTY_STAMP, &fail_code);

        let accepted_token = b
            .helper
            .acceptance_success(tok, &EMPTY_STAMP)
            .expect("Sending acceptance success failed");
        let started_token = b
            .helper
            .start_success(accepted_token, &[0, 1, 0, 1, 0, 1, 0])
            .expect("Sending start success failed");
        b.helper
            .completion_failure(started_token, fail_params)
            .expect("Completion failure");
        completion_fail_check(&mut b.helper.sender, req_id);
    }

    fn completion_success_check(sender: &mut TestSender, req_id: RequestId) {
        assert_eq!(sender.service_queue.borrow().len(), 3);
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
        let mut info = sender.service_queue.borrow_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);

        let cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 3,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: [0, 1, 0, 1, 0, 1, 0],
            },
            additional_data: None,
            req_id,
        };
        info = sender.service_queue.borrow_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);
        let cmp_info = TmInfo {
            common: CommonTmInfo {
                subservice: 7,
                apid: TEST_APID,
                msg_counter: 0,
                dest_id: 0,
                time_stamp: EMPTY_STAMP,
            },
            additional_data: None,
            req_id,
        };
        info = sender.service_queue.borrow_mut().pop_front().unwrap();
        assert_eq!(info, cmp_info);
    }

    #[test]
    fn test_complete_success_sequence() {
        let (b, tok) = base_init(false);
        let mut sender = TestSender::default();
        let accepted_token =
            b.vr.acceptance_success(tok, &sender, &EMPTY_STAMP)
                .expect("Sending acceptance success failed");
        let started_token =
            b.vr.start_success(accepted_token, &sender, &[0, 1, 0, 1, 0, 1, 0])
                .expect("Sending start success failed");
        b.vr.completion_success(started_token, &sender, &EMPTY_STAMP)
            .expect("Sending completion success failed");
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
        b.helper
            .completion_success(started_token, &EMPTY_STAMP)
            .expect("Sending completion success failed");
        completion_success_check(&mut b.helper.sender, tok.req_id);
    }

    #[test]
    fn test_seq_count_increment() {
        let pool_cfg =
            StaticPoolConfig::new(vec![(10, 32), (10, 64), (10, 128), (10, 1024)], false);
        let tm_pool = StaticMemoryPool::new(pool_cfg.clone());
        let shared_tm_store = SharedTmPool::new(tm_pool);
        let shared_tm_pool = shared_tm_store.clone_backing_pool();
        let (verif_tx, verif_rx) = mpsc::channel();
        let sender =
            TmInSharedPoolSenderWithId::new(0, "Verification Sender", shared_tm_store, verif_tx);
        let cfg = VerificationReporterCfg::new(TEST_APID, 1, 2, 8).unwrap();
        let mut reporter = VerificationReporterWithSender::new(&cfg, sender);

        let mut sph = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
        let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
        let pus_tc_0 = PusTcCreator::new_no_app_data(&mut sph, tc_header, true);
        let init_token = reporter.add_tc(&pus_tc_0);

        // Complete success sequence for a telecommand
        let accepted_token = reporter
            .acceptance_success(init_token, &EMPTY_STAMP)
            .unwrap();
        let started_token = reporter
            .start_success(accepted_token, &EMPTY_STAMP)
            .unwrap();
        reporter
            .completion_success(started_token, &EMPTY_STAMP)
            .unwrap();

        // Verify it arrives correctly on receiver end
        let mut tm_buf: [u8; 1024] = [0; 1024];
        let mut packet_idx = 0;
        while packet_idx < 3 {
            let addr = verif_rx.recv_timeout(Duration::from_millis(10)).unwrap();
            let tm_len;
            {
                let mut rg = shared_tm_pool.write().expect("Error locking shared pool");
                let store_guard = rg.read_with_guard(addr);
                tm_len = store_guard
                    .read(&mut tm_buf)
                    .expect("Error reading TM slice");
            }
            let (pus_tm, _) =
                PusTmReader::new(&tm_buf[0..tm_len], 7).expect("Error reading verification TM");
            if packet_idx == 0 {
                assert_eq!(pus_tm.subservice(), 1);
                assert_eq!(pus_tm.sp_header.seq_count(), 0);
            } else if packet_idx == 1 {
                assert_eq!(pus_tm.subservice(), 3);
                assert_eq!(pus_tm.sp_header.seq_count(), 0);
            } else if packet_idx == 2 {
                assert_eq!(pus_tm.subservice(), 7);
                assert_eq!(pus_tm.sp_header.seq_count(), 0);
            }
            packet_idx += 1;
        }
    }
}
