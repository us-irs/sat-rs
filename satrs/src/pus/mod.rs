//! # PUS support modules
//!
//! This module contains structures to make working with the PUS C standard easier.
//! The satrs-example application contains various usage examples of these components.
use crate::pool::{StoreAddr, StoreError};
use crate::pus::verification::{TcStateAccepted, TcStateToken, VerificationToken};
use crate::queue::{GenericReceiveError, GenericSendError};
use crate::request::{GenericMessage, RequestId};
use crate::ComponentId;
use core::fmt::{Display, Formatter};
use core::time::Duration;
#[cfg(feature = "alloc")]
use downcast_rs::{impl_downcast, Downcast};
#[cfg(feature = "alloc")]
use dyn_clone::DynClone;
#[cfg(feature = "std")]
use std::error::Error;

use spacepackets::ecss::tc::{PusTcCreator, PusTcReader};
use spacepackets::ecss::tm::PusTmCreator;
use spacepackets::ecss::PusError;
use spacepackets::{ByteConversionError, SpHeader};

pub mod action;
pub mod event;
pub mod event_man;
#[cfg(feature = "std")]
pub mod event_srv;
pub mod mode;
pub mod scheduler;
#[cfg(feature = "std")]
pub mod scheduler_srv;
#[cfg(feature = "std")]
pub mod test;
pub mod verification;

#[cfg(feature = "alloc")]
pub use alloc_mod::*;

#[cfg(feature = "std")]
pub use std_mod::*;

use self::verification::{TcStateStarted, VerificationReportingProvider};

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PusTmWrapper<'tm> {
    InStore(StoreAddr),
    Direct(PusTmCreator<'tm>),
}

impl From<StoreAddr> for PusTmWrapper<'_> {
    fn from(value: StoreAddr) -> Self {
        Self::InStore(value)
    }
}

impl<'tm> From<PusTmCreator<'tm>> for PusTmWrapper<'tm> {
    fn from(value: PusTmCreator<'tm>) -> Self {
        Self::Direct(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EcssTmtcError {
    Store(StoreError),
    ByteConversion(ByteConversionError),
    Pus(PusError),
    CantSendAddr(StoreAddr),
    CantSendDirectTm,
    Send(GenericSendError),
    Receive(GenericReceiveError),
}

impl Display for EcssTmtcError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            EcssTmtcError::Store(store) => {
                write!(f, "ecss tmtc error: {store}")
            }
            EcssTmtcError::ByteConversion(e) => {
                write!(f, "ecss tmtc error: {e}")
            }
            EcssTmtcError::Pus(e) => {
                write!(f, "ecss tmtc error: {e}")
            }
            EcssTmtcError::CantSendAddr(addr) => {
                write!(f, "can not send address {addr}")
            }
            EcssTmtcError::CantSendDirectTm => {
                write!(f, "can not send TM directly")
            }
            EcssTmtcError::Send(e) => {
                write!(f, "ecss tmtc error: {e}")
            }
            EcssTmtcError::Receive(e) => {
                write!(f, "ecss tmtc error {e}")
            }
        }
    }
}

impl From<StoreError> for EcssTmtcError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

impl From<PusError> for EcssTmtcError {
    fn from(value: PusError) -> Self {
        Self::Pus(value)
    }
}

impl From<GenericSendError> for EcssTmtcError {
    fn from(value: GenericSendError) -> Self {
        Self::Send(value)
    }
}

impl From<ByteConversionError> for EcssTmtcError {
    fn from(value: ByteConversionError) -> Self {
        Self::ByteConversion(value)
    }
}

impl From<GenericReceiveError> for EcssTmtcError {
    fn from(value: GenericReceiveError) -> Self {
        Self::Receive(value)
    }
}

#[cfg(feature = "std")]
impl Error for EcssTmtcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            EcssTmtcError::Store(e) => Some(e),
            EcssTmtcError::ByteConversion(e) => Some(e),
            EcssTmtcError::Pus(e) => Some(e),
            EcssTmtcError::Send(e) => Some(e),
            EcssTmtcError::Receive(e) => Some(e),
            _ => None,
        }
    }
}
pub trait EcssChannel: Send {
    /// Each sender can have an ID associated with it
    fn channel_id(&self) -> ComponentId;
    fn name(&self) -> &'static str {
        "unset"
    }
}

/// Generic trait for a user supplied sender object.
///
/// This sender object is responsible for sending PUS telemetry to a TM sink.
pub trait EcssTmSenderCore: Send {
    fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError>;
}

/// Generic trait for a user supplied sender object.
///
/// This sender object is responsible for sending PUS telecommands to a TC recipient. Each
/// telecommand can optionally have a token which contains its verification state.
pub trait EcssTcSenderCore {
    fn send_tc(&self, tc: PusTcCreator, token: Option<TcStateToken>) -> Result<(), EcssTmtcError>;
}

/// A PUS telecommand packet can be stored in memory using different methods. Right now,
/// storage inside a pool structure like [crate::pool::StaticMemoryPool], and storage inside a
/// `Vec<u8>` are supported.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TcInMemory {
    StoreAddr(StoreAddr),
    #[cfg(feature = "alloc")]
    Vec(alloc::vec::Vec<u8>),
}

impl From<StoreAddr> for TcInMemory {
    fn from(value: StoreAddr) -> Self {
        Self::StoreAddr(value)
    }
}

#[cfg(feature = "alloc")]
impl From<alloc::vec::Vec<u8>> for TcInMemory {
    fn from(value: alloc::vec::Vec<u8>) -> Self {
        Self::Vec(value)
    }
}

/// Generic structure for an ECSS PUS Telecommand and its correspoding verification token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcssTcAndToken {
    pub tc_in_memory: TcInMemory,
    pub token: Option<TcStateToken>,
}

impl EcssTcAndToken {
    pub fn new(tc_in_memory: impl Into<TcInMemory>, token: impl Into<TcStateToken>) -> Self {
        Self {
            tc_in_memory: tc_in_memory.into(),
            token: Some(token.into()),
        }
    }
}

/// Generic abstraction for a telecommand being sent around after is has been accepted.
pub struct AcceptedEcssTcAndToken {
    pub tc_in_memory: TcInMemory,
    pub token: VerificationToken<TcStateAccepted>,
}

impl From<AcceptedEcssTcAndToken> for EcssTcAndToken {
    fn from(value: AcceptedEcssTcAndToken) -> Self {
        EcssTcAndToken {
            tc_in_memory: value.tc_in_memory,
            token: Some(value.token.into()),
        }
    }
}

impl TryFrom<EcssTcAndToken> for AcceptedEcssTcAndToken {
    type Error = ();

    fn try_from(value: EcssTcAndToken) -> Result<Self, Self::Error> {
        if let Some(TcStateToken::Accepted(token)) = value.token {
            return Ok(AcceptedEcssTcAndToken {
                tc_in_memory: value.tc_in_memory,
                token,
            });
        }
        Err(())
    }
}

#[derive(Debug, Clone)]
pub enum TryRecvTmtcError {
    Tmtc(EcssTmtcError),
    Empty,
}

impl From<EcssTmtcError> for TryRecvTmtcError {
    fn from(value: EcssTmtcError) -> Self {
        Self::Tmtc(value)
    }
}

impl From<PusError> for TryRecvTmtcError {
    fn from(value: PusError) -> Self {
        Self::Tmtc(value.into())
    }
}

impl From<StoreError> for TryRecvTmtcError {
    fn from(value: StoreError) -> Self {
        Self::Tmtc(value.into())
    }
}

/// Generic trait for a user supplied receiver object.
pub trait EcssTcReceiverCore: EcssChannel {
    fn recv_tc(&self) -> Result<EcssTcAndToken, TryRecvTmtcError>;
}

/// Generic trait for objects which can receive ECSS PUS telecommands. This trait is
/// implemented by the [crate::tmtc::pus_distrib::PusDistributor] objects to allow passing PUS TC
/// packets into it. It is generally assumed that the telecommand is stored in some pool structure,
/// and the store address is passed as well. This allows efficient zero-copy forwarding of
/// telecommands.
pub trait ReceivesEcssPusTc {
    type Error;
    fn pass_pus_tc(&mut self, header: &SpHeader, pus_tc: &PusTcReader) -> Result<(), Self::Error>;
}

pub trait ActiveRequestMapProvider<V>: Sized {
    fn insert(&mut self, request_id: &RequestId, request_info: V);
    fn get(&self, request_id: RequestId) -> Option<&V>;
    fn get_mut(&mut self, request_id: RequestId) -> Option<&mut V>;
    fn remove(&mut self, request_id: RequestId) -> bool;

    /// Call a user-supplied closure for each active request.
    fn for_each<F: FnMut(&RequestId, &V)>(&self, f: F);

    /// Call a user-supplied closure for each active request. Mutable variant.
    fn for_each_mut<F: FnMut(&RequestId, &mut V)>(&mut self, f: F);
}

pub trait ActiveRequestProvider {
    fn target_id(&self) -> ComponentId;
    fn token(&self) -> VerificationToken<TcStateStarted>;
    fn has_timed_out(&self) -> bool;
    fn timeout(&self) -> Duration;
}

/// This trait is an abstraction for the routing of PUS request to a dedicated
/// recipient using the generic [TargetId].
pub trait PusRequestRouter<Request> {
    type Error;

    fn route(
        &self,
        target_id: ComponentId,
        request_id: RequestId,
        request: Request,
        token: VerificationToken<TcStateStarted>,
    ) -> Result<(), Self::Error>;
}

pub trait PusReplyHandler<ActiveRequestInfo: ActiveRequestProvider, ReplyType> {
    type Error;

    /// This function handles a reply for a given PUS request and returns whether that request
    /// is finished. A finished PUS request will be removed from the active request map.
    fn handle_reply(
        &mut self,
        reply: &GenericMessage<ReplyType>,
        active_request: &ActiveRequestInfo,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
        tm_sender: &impl EcssTmSenderCore,
    ) -> Result<bool, Self::Error>;

    fn handle_unexpected_reply(
        &mut self,
        reply: &GenericMessage<ReplyType>,
        tm_sender: &impl EcssTmSenderCore,
    ) -> Result<(), Self::Error>;

    /// Handle the timeout of an active request.
    fn handle_request_timeout(
        &mut self,
        active_request: &ActiveRequestInfo,
        verification_handler: &impl VerificationReportingProvider,
        time_stamp: &[u8],
        tm_sender: &impl EcssTmSenderCore,
    ) -> Result<(), Self::Error>;
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use hashbrown::HashMap;

    use super::*;

    use crate::pus::verification::VerificationReportingProvider;

    /// Extension trait for [EcssTmSenderCore].
    ///
    /// It provides additional functionality, for example by implementing the [Downcast] trait
    /// and the [DynClone] trait.
    ///
    /// [Downcast] is implemented to allow passing the sender as a boxed trait object and still
    /// retrieve the concrete type at a later point.
    ///
    /// [DynClone] allows cloning the trait object as long as the boxed object implements
    /// [Clone].
    #[cfg(feature = "alloc")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
    pub trait EcssTmSender: EcssTmSenderCore + Downcast + DynClone {
        // Remove this once trait upcasting coercion has been implemented.
        // Tracking issue: https://github.com/rust-lang/rust/issues/65991
        fn upcast(&self) -> &dyn EcssTmSenderCore;
        // Remove this once trait upcasting coercion has been implemented.
        // Tracking issue: https://github.com/rust-lang/rust/issues/65991
        fn upcast_mut(&mut self) -> &mut dyn EcssTmSenderCore;
    }

    /// Blanket implementation for all types which implement [EcssTmSenderCore] and are clonable.
    impl<T> EcssTmSender for T
    where
        T: EcssTmSenderCore + Clone + 'static,
    {
        // Remove this once trait upcasting coercion has been implemented.
        // Tracking issue: https://github.com/rust-lang/rust/issues/65991
        fn upcast(&self) -> &dyn EcssTmSenderCore {
            self
        }
        // Remove this once trait upcasting coercion has been implemented.
        // Tracking issue: https://github.com/rust-lang/rust/issues/65991
        fn upcast_mut(&mut self) -> &mut dyn EcssTmSenderCore {
            self
        }
    }

    dyn_clone::clone_trait_object!(EcssTmSender);
    impl_downcast!(EcssTmSender);

    /// Extension trait for [EcssTcSenderCore].
    ///
    /// It provides additional functionality, for example by implementing the [Downcast] trait
    /// and the [DynClone] trait.
    ///
    /// [Downcast] is implemented to allow passing the sender as a boxed trait object and still
    /// retrieve the concrete type at a later point.
    ///
    /// [DynClone] allows cloning the trait object as long as the boxed object implements
    /// [Clone].
    #[cfg(feature = "alloc")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
    pub trait EcssTcSender: EcssTcSenderCore + Downcast + DynClone {}

    /// Blanket implementation for all types which implement [EcssTcSenderCore] and are clonable.
    impl<T> EcssTcSender for T where T: EcssTcSenderCore + Clone + 'static {}

    dyn_clone::clone_trait_object!(EcssTcSender);
    impl_downcast!(EcssTcSender);

    /// Extension trait for [EcssTcReceiverCore].
    ///
    /// It provides additional functionality, for example by implementing the [Downcast] trait
    /// and the [DynClone] trait.
    ///
    /// [Downcast] is implemented to allow passing the sender as a boxed trait object and still
    /// retrieve the concrete type at a later point.
    ///
    /// [DynClone] allows cloning the trait object as long as the boxed object implements
    /// [Clone].
    #[cfg(feature = "alloc")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
    pub trait EcssTcReceiver: EcssTcReceiverCore + Downcast {}

    /// Blanket implementation for all types which implement [EcssTcReceiverCore] and are clonable.
    impl<T> EcssTcReceiver for T where T: EcssTcReceiverCore + 'static {}

    impl_downcast!(EcssTcReceiver);

    /// This trait is an abstraction for the conversion of a PUS telecommand into a generic request
    /// type.
    ///
    /// Having a dedicated trait for this allows maximum flexiblity and tailoring of the standard.
    /// The only requirement is that a valid [TargetId] and a request instance are returned by the
    /// core conversion function.
    ///
    /// The user should take care of performing the error handling as well. Some of the following
    /// aspects might be relevant:
    ///
    /// - Checking the validity of the APID, service ID, subservice ID.
    /// - Checking the validity of the user data.
    ///
    /// A [VerificationReportingProvider] instance is passed to the user to also allow handling
    /// of the verification process as part of the PUS standard requirements.
    pub trait PusTcToRequestConverter<ActiveRequestInfo: ActiveRequestProvider, Request> {
        type Error;
        fn convert(
            &mut self,
            token: VerificationToken<TcStateAccepted>,
            tc: &PusTcReader,
            time_stamp: &[u8],
            verif_reporter: &impl VerificationReportingProvider,
        ) -> Result<(ActiveRequestInfo, Request), Self::Error>;
    }

    #[derive(Clone, Debug)]
    pub struct DefaultActiveRequestMap<V>(pub HashMap<RequestId, V>);

    impl<V> Default for DefaultActiveRequestMap<V> {
        fn default() -> Self {
            Self(HashMap::new())
        }
    }

    impl<V> ActiveRequestMapProvider<V> for DefaultActiveRequestMap<V> {
        fn insert(&mut self, request_id: &RequestId, request: V) {
            self.0.insert(*request_id, request);
        }

        fn get(&self, request_id: RequestId) -> Option<&V> {
            self.0.get(&request_id)
        }

        fn get_mut(&mut self, request_id: RequestId) -> Option<&mut V> {
            self.0.get_mut(&request_id)
        }

        fn remove(&mut self, request_id: RequestId) -> bool {
            self.0.remove(&request_id).is_some()
        }

        fn for_each<F: FnMut(&RequestId, &V)>(&self, mut f: F) {
            for (req_id, active_req) in &self.0 {
                f(req_id, active_req);
            }
        }

        fn for_each_mut<F: FnMut(&RequestId, &mut V)>(&mut self, mut f: F) {
            for (req_id, active_req) in &mut self.0 {
                f(req_id, active_req);
            }
        }
    }

    /*
    /// Generic reply handler structure which can be used to handle replies for a specific PUS
    /// service.
    ///
    /// This is done by keeping track of active requests using an internal map structure. An API
    /// to register new active requests is exposed as well.
    /// The reply handler performs boilerplate tasks like performing the verification handling and
    /// timeout handling.
    ///
    /// This object is not useful by itself but serves as a common building block for high-level
    /// PUS reply handlers. Concrete PUS handlers should constrain the [ActiveRequestProvider] and
    /// the `ReplyType` generics to specific types tailored towards PUS services in addition to
    /// providing an API which can process received replies and convert them into verification
    /// completions or other operation like user hook calls. The framework also provides some
    /// concrete PUS handlers for common PUS services like the mode, action and housekeeping
    /// service.
    ///
    /// This object does not automatically update its internal time information used to check for
    /// timeouts. The user should call the [Self::update_time] and [Self::update_time_from_now]
    /// methods to do this.
    pub struct PusServiceReplyHandler<
        ActiveRequestMap: ActiveRequestMapProvider<ActiveRequestType>,
        ReplyHook: ReplyHandlerHook<ActiveRequestType, ReplyType>,
        ActiveRequestType: ActiveRequestProvider,
        ReplyType,
    > {
        pub active_request_map: ActiveRequestMap,
        pub tm_buf: alloc::vec::Vec<u8>,
        pub current_time: UnixTimestamp,
        pub user_hook: ReplyHook,
        phantom: PhantomData<(ActiveRequestType, ReplyType)>,
    }

    impl<
            ActiveRequestMap: ActiveRequestMapProvider<ActiveRequestType>,
            ReplyHook: ReplyHandlerHook<ActiveRequestType, ReplyType>,
            ActiveRequestType: ActiveRequestProvider,
            ReplyType,
        >
        PusServiceReplyHandler<
            ActiveRequestMap,
            ReplyHook,
            ActiveRequestType,
            ReplyType,
        >
    {
        #[cfg(feature = "std")]
        #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
        pub fn new_from_now(
            active_request_map: ActiveRequestMap,
            fail_data_buf_size: usize,
            user_hook: ReplyHook,
        ) -> Result<Self, std::time::SystemTimeError> {
            let current_time = UnixTimestamp::from_now()?;
            Ok(Self::new(
                active_request_map,
                fail_data_buf_size,
                user_hook,
                tm_sender,
                current_time,
            ))
        }

        pub fn new(
            active_request_map: ActiveRequestMap,
            fail_data_buf_size: usize,
            user_hook: ReplyHook,
            tm_sender: TmSender,
            init_time: UnixTimestamp,
        ) -> Self {
            Self {
                active_request_map,
                tm_buf: alloc::vec![0; fail_data_buf_size],
                current_time: init_time,
                user_hook,
                tm_sender,
                phantom: PhantomData,
            }
        }

        pub fn add_routed_request(
            &mut self,
            request_id: verification::RequestId,
            active_request_type: ActiveRequestType,
        ) {
            self.active_request_map
                .insert(&request_id.into(), active_request_type);
        }

        pub fn request_active(&self, request_id: RequestId) -> bool {
            self.active_request_map.get(request_id).is_some()
        }

        /// Check for timeouts across all active requests.
        ///
        /// It will call [Self::handle_timeout] for all active requests which have timed out.
        pub fn check_for_timeouts(&mut self, time_stamp: &[u8]) -> Result<(), EcssTmtcError> {
            let mut timed_out_commands = alloc::vec::Vec::new();
            self.active_request_map.for_each(|request_id, active_req| {
                let diff = self.current_time - active_req.start_time();
                if diff.duration_absolute > active_req.timeout() {
                    self.handle_timeout(active_req, time_stamp);
                }
                timed_out_commands.push(*request_id);
            });
            for timed_out_command in timed_out_commands {
                self.active_request_map.remove(timed_out_command);
            }
            Ok(())
        }

        /// Handle the timeout for a given active request.
        ///
        /// This implementation will report a verification completion failure with a user-provided
        /// error code. It supplies the configured request timeout in milliseconds as a [u64]
        /// serialized in big-endian format as the failure data.
        pub fn handle_timeout(&self, active_request: &ActiveRequestType, time_stamp: &[u8]) {
            let timeout = active_request.timeout().as_millis() as u64;
            let timeout_raw = timeout.to_be_bytes();
            self.verification_reporter
                .completion_failure(
                    active_request.token(),
                    FailParams::new(
                        time_stamp,
                        &self.user_hook.timeout_error_code(),
                        &timeout_raw,
                    ),
                )
                .unwrap();
            self.user_hook.timeout_callback(active_request);
        }

        /// Update the current time used for timeout checks based on the current OS time.
        #[cfg(feature = "std")]
        #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
        pub fn update_time_from_now(&mut self) -> Result<(), std::time::SystemTimeError> {
            self.current_time = UnixTimestamp::from_now()?;
            Ok(())
        }

        /// Update the current time used for timeout checks.
        pub fn update_time(&mut self, time: UnixTimestamp) {
            self.current_time = time;
        }
    }
    */
}

#[cfg(feature = "std")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub mod std_mod {
    use crate::pool::{
        PoolProvider, PoolProviderWithGuards, SharedStaticMemoryPool, StoreAddr, StoreError,
    };
    use crate::pus::verification::{TcStateAccepted, VerificationToken};
    use crate::pus::{
        EcssChannel, EcssTcAndToken, EcssTcReceiverCore, EcssTmSenderCore, EcssTmtcError,
        GenericReceiveError, GenericSendError, PusTmWrapper, TryRecvTmtcError,
    };
    use crate::tmtc::tm_helper::SharedTmPool;
    use crate::ComponentId;
    use alloc::vec::Vec;
    use core::time::Duration;
    use spacepackets::ecss::tc::PusTcReader;
    use spacepackets::ecss::tm::PusTmCreator;
    use spacepackets::ecss::WritablePusPacket;
    use spacepackets::time::StdTimestampError;
    use spacepackets::ByteConversionError;
    use std::println;
    use std::string::String;
    use std::sync::mpsc;
    use std::sync::mpsc::TryRecvError;
    use thiserror::Error;

    #[cfg(feature = "crossbeam")]
    pub use cb_mod::*;

    use super::verification::{TcStateStarted, VerificationReportingProvider};
    use super::{AcceptedEcssTcAndToken, ActiveRequestProvider, TcInMemory};

    impl From<mpsc::SendError<StoreAddr>> for EcssTmtcError {
        fn from(_: mpsc::SendError<StoreAddr>) -> Self {
            Self::Send(GenericSendError::RxDisconnected)
        }
    }

    impl EcssTmSenderCore for mpsc::Sender<StoreAddr> {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmWrapper::InStore(addr) => self
                    .send(addr)
                    .map_err(|_| GenericSendError::RxDisconnected)?,
                PusTmWrapper::Direct(_) => return Err(EcssTmtcError::CantSendDirectTm),
            };
            Ok(())
        }
    }

    impl EcssTmSenderCore for mpsc::SyncSender<StoreAddr> {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmWrapper::InStore(addr) => self
                    .try_send(addr)
                    .map_err(|e| EcssTmtcError::Send(e.into()))?,
                PusTmWrapper::Direct(_) => return Err(EcssTmtcError::CantSendDirectTm),
            };
            Ok(())
        }
    }

    impl EcssTmSenderCore for mpsc::Sender<Vec<u8>> {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmWrapper::InStore(addr) => return Err(EcssTmtcError::CantSendAddr(addr)),
                PusTmWrapper::Direct(tm) => self
                    .send(tm.to_vec()?)
                    .map_err(|e| EcssTmtcError::Send(e.into()))?,
            };
            Ok(())
        }
    }

    impl EcssTmSenderCore for mpsc::SyncSender<Vec<u8>> {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            match tm {
                PusTmWrapper::InStore(addr) => return Err(EcssTmtcError::CantSendAddr(addr)),
                PusTmWrapper::Direct(tm) => self
                    .send(tm.to_vec()?)
                    .map_err(|e| EcssTmtcError::Send(e.into()))?,
            };
            Ok(())
        }
    }

    #[derive(Clone)]
    pub struct TmInSharedPoolSenderWithId<Sender: EcssTmSenderCore> {
        channel_id: ComponentId,
        name: &'static str,
        shared_tm_store: SharedTmPool,
        sender: Sender,
    }

    impl<Sender: EcssTmSenderCore> EcssChannel for TmInSharedPoolSenderWithId<Sender> {
        fn channel_id(&self) -> ComponentId {
            self.channel_id
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl<Sender: EcssTmSenderCore> TmInSharedPoolSenderWithId<Sender> {
        pub fn send_direct_tm(&self, tm: PusTmCreator) -> Result<(), EcssTmtcError> {
            let addr = self.shared_tm_store.add_pus_tm(&tm)?;
            self.sender.send_tm(PusTmWrapper::InStore(addr))
        }
    }

    impl<Sender: EcssTmSenderCore> EcssTmSenderCore for TmInSharedPoolSenderWithId<Sender> {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            if let PusTmWrapper::Direct(tm) = tm {
                return self.send_direct_tm(tm);
            }
            self.sender.send_tm(tm)
        }
    }

    impl<Sender: EcssTmSenderCore> TmInSharedPoolSenderWithId<Sender> {
        pub fn new(
            id: ComponentId,
            name: &'static str,
            shared_tm_store: SharedTmPool,
            sender: Sender,
        ) -> Self {
            Self {
                channel_id: id,
                name,
                shared_tm_store,
                sender,
            }
        }
    }

    pub type TmInSharedPoolSenderWithMpsc = TmInSharedPoolSenderWithId<mpsc::Sender<StoreAddr>>;
    pub type TmInSharedPoolSenderWithBoundedMpsc =
        TmInSharedPoolSenderWithId<mpsc::SyncSender<StoreAddr>>;

    /// This class can be used if frequent heap allocations during run-time are not an issue.
    /// PUS TM packets will be sent around as [Vec]s. Please note that the current implementation
    /// of this class can not deal with store addresses, so it is assumed that is is always
    /// going to be called with direct packets.
    #[derive(Clone)]
    pub struct TmAsVecSenderWithId<Sender: EcssTmSenderCore> {
        id: ComponentId,
        name: &'static str,
        sender: Sender,
    }

    impl From<mpsc::SendError<Vec<u8>>> for EcssTmtcError {
        fn from(_: mpsc::SendError<Vec<u8>>) -> Self {
            Self::Send(GenericSendError::RxDisconnected)
        }
    }

    impl<Sender: EcssTmSenderCore> TmAsVecSenderWithId<Sender> {
        pub fn new(id: ComponentId, name: &'static str, sender: Sender) -> Self {
            Self { id, sender, name }
        }
    }

    impl<Sender: EcssTmSenderCore> EcssChannel for TmAsVecSenderWithId<Sender> {
        fn channel_id(&self) -> ComponentId {
            self.id
        }
        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl<Sender: EcssTmSenderCore> EcssTmSenderCore for TmAsVecSenderWithId<Sender> {
        fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
            self.sender.send_tm(tm)
        }
    }

    pub type TmAsVecSenderWithMpsc = TmAsVecSenderWithId<mpsc::Sender<Vec<u8>>>;
    pub type TmAsVecSenderWithBoundedMpsc = TmAsVecSenderWithId<mpsc::SyncSender<Vec<u8>>>;

    pub struct MpscTcReceiver {
        id: ComponentId,
        name: &'static str,
        receiver: mpsc::Receiver<EcssTcAndToken>,
    }

    impl EcssChannel for MpscTcReceiver {
        fn channel_id(&self) -> ComponentId {
            self.id
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    impl EcssTcReceiverCore for MpscTcReceiver {
        fn recv_tc(&self) -> Result<EcssTcAndToken, TryRecvTmtcError> {
            self.receiver.try_recv().map_err(|e| match e {
                TryRecvError::Empty => TryRecvTmtcError::Empty,
                TryRecvError::Disconnected => TryRecvTmtcError::Tmtc(EcssTmtcError::from(
                    GenericReceiveError::TxDisconnected(Some(self.channel_id())),
                )),
            })
        }
    }

    impl MpscTcReceiver {
        pub fn new(
            id: ComponentId,
            name: &'static str,
            receiver: mpsc::Receiver<EcssTcAndToken>,
        ) -> Self {
            Self { id, name, receiver }
        }
    }

    #[cfg(feature = "crossbeam")]
    pub mod cb_mod {
        use super::*;
        use crossbeam_channel as cb;

        pub type TmInSharedPoolSenderWithCrossbeam =
            TmInSharedPoolSenderWithId<cb::Sender<StoreAddr>>;

        impl From<cb::SendError<StoreAddr>> for EcssTmtcError {
            fn from(_: cb::SendError<StoreAddr>) -> Self {
                Self::Send(GenericSendError::RxDisconnected)
            }
        }

        impl From<cb::TrySendError<StoreAddr>> for EcssTmtcError {
            fn from(value: cb::TrySendError<StoreAddr>) -> Self {
                match value {
                    cb::TrySendError::Full(_) => Self::Send(GenericSendError::QueueFull(None)),
                    cb::TrySendError::Disconnected(_) => {
                        Self::Send(GenericSendError::RxDisconnected)
                    }
                }
            }
        }

        impl EcssTmSenderCore for cb::Sender<StoreAddr> {
            fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
                match tm {
                    PusTmWrapper::InStore(addr) => self
                        .try_send(addr)
                        .map_err(|e| EcssTmtcError::Send(e.into()))?,
                    PusTmWrapper::Direct(_) => return Err(EcssTmtcError::CantSendDirectTm),
                };
                Ok(())
            }
        }
        impl EcssTmSenderCore for cb::Sender<Vec<u8>> {
            fn send_tm(&self, tm: PusTmWrapper) -> Result<(), EcssTmtcError> {
                match tm {
                    PusTmWrapper::InStore(addr) => return Err(EcssTmtcError::CantSendAddr(addr)),
                    PusTmWrapper::Direct(tm) => self
                        .send(tm.to_vec()?)
                        .map_err(|e| EcssTmtcError::Send(e.into()))?,
                };
                Ok(())
            }
        }

        pub struct CrossbeamTcReceiver {
            id: ComponentId,
            name: &'static str,
            receiver: cb::Receiver<EcssTcAndToken>,
        }

        impl CrossbeamTcReceiver {
            pub fn new(
                id: ComponentId,
                name: &'static str,
                receiver: cb::Receiver<EcssTcAndToken>,
            ) -> Self {
                Self { id, name, receiver }
            }
        }

        impl EcssChannel for CrossbeamTcReceiver {
            fn channel_id(&self) -> ComponentId {
                self.id
            }

            fn name(&self) -> &'static str {
                self.name
            }
        }

        impl EcssTcReceiverCore for CrossbeamTcReceiver {
            fn recv_tc(&self) -> Result<EcssTcAndToken, TryRecvTmtcError> {
                self.receiver.try_recv().map_err(|e| match e {
                    cb::TryRecvError::Empty => TryRecvTmtcError::Empty,
                    cb::TryRecvError::Disconnected => TryRecvTmtcError::Tmtc(EcssTmtcError::from(
                        GenericReceiveError::TxDisconnected(Some(self.channel_id())),
                    )),
                })
            }
        }
    }

    /// This is a high-level handler for the generic PUS services which need to convert PUS
    /// commands into a request/reply pattern.
    ///
    /// It performs the following handling steps:
    ///
    /// 1. Retrieve the next TC packet from the [PusServiceHelper]. The [EcssTcInMemConverter]
    ///    allows to configure the used telecommand memory backend.
    /// 2. Convert the TC to a targeted action request using the provided
    ///    [PusTcToRequestConverter]. The generic error type is constrained to the
    ///    [PusPacketHandlingError] for the concrete implementation which offers a packet handler.
    /// 3. Route the action request using the provided [PusRequestRouter].
    /*
    pub struct PusTargetedRequestHandler<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
        RequestConverter: PusTcToRequestConverter<Request>,
        RequestRouter: PusRequestRouter<Request, Error = RoutingError>,
        Request,
        RoutingError = GenericRoutingError,
    > {
        service_helper:
            PusServiceHelper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>,
        pub request_converter: RequestConverter,
        pub request_router: RequestRouter,
        phantom: PhantomData<Request>,
    }

    // pub trait PusReplyHandlerProvider {
    // fn add_routed_request(&mut self, request_id: RequestId, active_request: ActiveRequest);
    // }
    impl<
            TcReceiver: EcssTcReceiverCore,
            TmSender: EcssTmSenderCore,
            TcInMemConverter: EcssTcInMemConverter,
            VerificationReporter: VerificationReportingProvider,
            RequestConverter: PusTcToRequestConverter<Request, Error = PusPacketHandlingError>,
            RequestRouter: PusRequestRouter<Request, Error = RoutingError>,
            Request,
            RoutingError: Clone,
        >
        PusTargetedRequestHandler<
            TcReceiver,
            TmSender,
            TcInMemConverter,
            VerificationReporter,
            RequestConverter,
            RequestRouter,
            Request,
            RoutingError,
        >
    where
        PusPacketHandlingError: From<RoutingError>,
    {
        pub fn new(
            service_helper: PusServiceHelper<
                TcReceiver,
                TmSender,
                TcInMemConverter,
                VerificationReporter,
            >,
            request_converter: RequestConverter,
            request_router: RequestRouter,
        ) -> Self {
            Self {
                service_helper,
                request_converter,
                request_router,
                phantom: PhantomData,
            }
        }

        /// Core function to poll the next TC packet and try to handle it.
        pub fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError> {
            let possible_packet = self.service_helper.retrieve_and_accept_next_packet()?;
            if possible_packet.is_none() {
                return Ok(PusPacketHandlerResult::Empty);
            }
            let ecss_tc_and_token = possible_packet.unwrap();
            let tc = self
                .service_helper
                .tc_in_mem_converter
                .convert_ecss_tc_in_memory_to_reader(&ecss_tc_and_token.tc_in_memory)?;
            let mut partial_error = None;
            let time_stamp = get_current_cds_short_timestamp(&mut partial_error);
            let (target_id, action_request) = self.request_converter.convert(
                ecss_tc_and_token.token,
                &tc,
                &time_stamp,
                &self.service_helper.common.verification_handler,
            )?;
            if let Err(e) =
                self.request_router
                    .route(target_id, action_request, ecss_tc_and_token.token)
            {
                self.request_router.handle_error(
                    target_id,
                    ecss_tc_and_token.token,
                    &tc,
                    e.clone(),
                    &time_stamp,
                    &self.service_helper.common.verification_handler,
                );
                return Err(e.into());
            }
            Ok(PusPacketHandlerResult::RequestHandled)
        }
    }
    */

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ActivePusRequestStd {
        target_id: ComponentId,
        token: VerificationToken<TcStateStarted>,
        start_time: std::time::Instant,
        timeout: Duration,
    }

    impl ActivePusRequestStd {
        pub fn new(
            target_id: ComponentId,
            token: VerificationToken<TcStateStarted>,
            timeout: Duration,
        ) -> Self {
            Self {
                target_id,
                token,
                start_time: std::time::Instant::now(),
                timeout,
            }
        }
    }

    impl ActiveRequestProvider for ActivePusRequestStd {
        fn target_id(&self) -> ComponentId {
            self.target_id
        }
        fn token(&self) -> VerificationToken<TcStateStarted> {
            self.token
        }

        fn timeout(&self) -> Duration {
            self.timeout
        }

        fn has_timed_out(&self) -> bool {
            std::time::Instant::now() - self.start_time > self.timeout
        }
    }

    // TODO: All these types could probably be no_std if we implemented error handling ourselves..
    // but thiserror is really nice, so keep it like this for simplicity for now. Maybe thiserror
    // will be no_std soon, see https://github.com/rust-lang/rust/issues/103765 .

    #[derive(Debug, Clone, Error)]
    pub enum PusTcFromMemError {
        #[error("generic PUS error: {0}")]
        EcssTmtc(#[from] EcssTmtcError),
        #[error("invalid format of TC in memory: {0:?}")]
        InvalidFormat(TcInMemory),
    }

    #[derive(Debug, Clone, Error)]
    pub enum GenericRoutingError {
        // #[error("not enough application data, expected at least {expected}, found {found}")]
        // NotEnoughAppData { expected: usize, found: usize },
        #[error("Unknown target ID {0}")]
        UnknownTargetId(ComponentId),
        #[error("Sending action request failed: {0}")]
        Send(GenericSendError),
    }

    /// This error can be used for generic conversions from PUS Telecommands to request types.
    ///
    /// Please note that this error can also be used if no request is generated and the PUS
    /// service, subservice and application data is used directly to perform some request.
    #[derive(Debug, Clone, Error)]
    pub enum GenericConversionError {
        #[error("wrong service number {0} for packet handler")]
        WrongService(u8),
        #[error("invalid subservice {0}")]
        InvalidSubservice(u8),
        #[error("not enough application data, expected at least {expected}, found {found}")]
        NotEnoughAppData { expected: usize, found: usize },
        #[error("invalid application data")]
        InvalidAppData(String),
    }

    /// Wrapper type which tries to encapsulate all possible errors when handling PUS packets.
    #[derive(Debug, Clone, Error)]
    pub enum PusPacketHandlingError {
        #[error("error polling PUS TC packet: {0}")]
        TcPolling(#[from] EcssTmtcError),
        #[error("error generating PUS reader from memory: {0}")]
        TcFromMem(#[from] PusTcFromMemError),
        #[error("generic request conversion error: {0}")]
        RequestConversion(#[from] GenericConversionError),
        #[error("request routing error: {0}")]
        RequestRouting(#[from] GenericRoutingError),
        #[error("invalid verification token")]
        InvalidVerificationToken,
        #[error("other error {0}")]
        Other(String),
    }

    #[derive(Debug, Clone, Error)]
    pub enum PartialPusHandlingError {
        #[error("generic timestamp generation error")]
        Time(#[from] StdTimestampError),
        #[error("error sending telemetry: {0}")]
        TmSend(#[from] EcssTmtcError),
        #[error("error sending verification message")]
        Verification,
        #[error("invalid verification token")]
        NoVerificationToken,
    }

    /// Generic result type for handlers which can process PUS packets.
    #[derive(Debug, Clone)]
    pub enum PusPacketHandlerResult {
        RequestHandled,
        RequestHandledPartialSuccess(PartialPusHandlingError),
        SubserviceNotImplemented(u8, VerificationToken<TcStateAccepted>),
        CustomSubservice(u8, VerificationToken<TcStateAccepted>),
        Empty,
    }

    impl From<PartialPusHandlingError> for PusPacketHandlerResult {
        fn from(value: PartialPusHandlingError) -> Self {
            Self::RequestHandledPartialSuccess(value)
        }
    }

    pub trait EcssTcInMemConverter {
        fn cache(&mut self, possible_packet: &TcInMemory) -> Result<(), PusTcFromMemError>;

        fn tc_slice_raw(&self) -> &[u8];

        fn cache_and_convert(
            &mut self,
            possible_packet: &TcInMemory,
        ) -> Result<PusTcReader<'_>, PusTcFromMemError> {
            self.cache(possible_packet)?;
            Ok(PusTcReader::new(self.tc_slice_raw())
                .map_err(EcssTmtcError::Pus)?
                .0)
        }

        fn convert(&self) -> Result<PusTcReader<'_>, PusTcFromMemError> {
            Ok(PusTcReader::new(self.tc_slice_raw())
                .map_err(EcssTmtcError::Pus)?
                .0)
        }
    }

    /// Converter structure for PUS telecommands which are stored inside a `Vec<u8>` structure.
    /// Please note that this structure is not able to convert TCs which are stored inside a
    /// [SharedStaticMemoryPool].
    #[derive(Default, Clone)]
    pub struct EcssTcInVecConverter {
        pub pus_tc_raw: Option<Vec<u8>>,
    }

    impl EcssTcInMemConverter for EcssTcInVecConverter {
        fn cache(&mut self, tc_in_memory: &TcInMemory) -> Result<(), PusTcFromMemError> {
            self.pus_tc_raw = None;
            match tc_in_memory {
                super::TcInMemory::StoreAddr(_) => {
                    return Err(PusTcFromMemError::InvalidFormat(tc_in_memory.clone()));
                }
                super::TcInMemory::Vec(vec) => {
                    self.pus_tc_raw = Some(vec.clone());
                }
            };
            Ok(())
        }

        fn tc_slice_raw(&self) -> &[u8] {
            if self.pus_tc_raw.is_none() {
                return &[];
            }
            self.pus_tc_raw.as_ref().unwrap()
        }
    }

    /// Converter structure for PUS telecommands which are stored inside
    /// [SharedStaticMemoryPool] structure. This is useful if run-time allocation for these
    /// packets should be avoided. Please note that this structure is not able to convert TCs which
    /// are stored as a `Vec<u8>`.
    pub struct EcssTcInSharedStoreConverter {
        shared_tc_store: SharedStaticMemoryPool,
        pus_buf: Vec<u8>,
    }

    impl EcssTcInSharedStoreConverter {
        pub fn new(shared_tc_store: SharedStaticMemoryPool, max_expected_tc_size: usize) -> Self {
            Self {
                shared_tc_store,
                pus_buf: alloc::vec![0; max_expected_tc_size],
            }
        }

        pub fn copy_tc_to_buf(&mut self, addr: StoreAddr) -> Result<(), PusTcFromMemError> {
            // Keep locked section as short as possible.
            let mut tc_pool = self.shared_tc_store.write().map_err(|_| {
                PusTcFromMemError::EcssTmtc(EcssTmtcError::Store(StoreError::LockError))
            })?;
            let tc_size = tc_pool.len_of_data(&addr).map_err(EcssTmtcError::Store)?;
            if tc_size > self.pus_buf.len() {
                return Err(
                    EcssTmtcError::ByteConversion(ByteConversionError::ToSliceTooSmall {
                        found: self.pus_buf.len(),
                        expected: tc_size,
                    })
                    .into(),
                );
            }
            let tc_guard = tc_pool.read_with_guard(addr);
            // TODO: Proper error handling.
            tc_guard.read(&mut self.pus_buf[0..tc_size]).unwrap();
            Ok(())
        }
    }

    impl EcssTcInMemConverter for EcssTcInSharedStoreConverter {
        fn cache(&mut self, tc_in_memory: &TcInMemory) -> Result<(), PusTcFromMemError> {
            match tc_in_memory {
                super::TcInMemory::StoreAddr(addr) => {
                    self.copy_tc_to_buf(*addr)?;
                }
                super::TcInMemory::Vec(_) => {
                    return Err(PusTcFromMemError::InvalidFormat(tc_in_memory.clone()));
                }
            };
            Ok(())
        }

        fn tc_slice_raw(&self) -> &[u8] {
            self.pus_buf.as_ref()
        }
    }

    pub struct PusServiceBase<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        VerificationReporter: VerificationReportingProvider,
    > {
        pub tc_receiver: TcReceiver,
        pub tm_sender: TmSender,
        pub tm_apid: u16,
        pub verif_reporter: VerificationReporter,
    }

    /// This is a high-level PUS packet handler helper.
    ///
    /// It performs some of the boilerplate acitivities involved when handling PUS telecommands and
    /// it can be used to implement the handling of PUS telecommands for certain PUS telecommands
    /// groups (for example individual services).
    ///
    /// This base class can handle PUS telecommands backed by different memory storage machanisms
    /// by using the [EcssTcInMemConverter] abstraction. This object provides some convenience
    /// methods to make the generic parts of TC handling easier.
    pub struct PusServiceHelper<
        TcReceiver: EcssTcReceiverCore,
        TmSender: EcssTmSenderCore,
        TcInMemConverter: EcssTcInMemConverter,
        VerificationReporter: VerificationReportingProvider,
    > {
        pub common: PusServiceBase<TcReceiver, TmSender, VerificationReporter>,
        pub tc_in_mem_converter: TcInMemConverter,
    }

    impl<
            TcReceiver: EcssTcReceiverCore,
            TmSender: EcssTmSenderCore,
            TcInMemConverter: EcssTcInMemConverter,
            VerificationReporter: VerificationReportingProvider,
        > PusServiceHelper<TcReceiver, TmSender, TcInMemConverter, VerificationReporter>
    {
        pub fn new(
            tc_receiver: TcReceiver,
            tm_sender: TmSender,
            tm_apid: u16,
            verification_handler: VerificationReporter,
            tc_in_mem_converter: TcInMemConverter,
        ) -> Self {
            Self {
                common: PusServiceBase {
                    tc_receiver,
                    tm_sender,
                    tm_apid,
                    verif_reporter: verification_handler,
                },
                tc_in_mem_converter,
            }
        }

        /// This function can be used to poll the internal [EcssTcReceiverCore] object for the next
        /// telecommand packet. It will return `Ok(None)` if there are not packets available.
        /// In any other case, it will perform the acceptance of the ECSS TC packet using the
        /// internal [VerificationReportingProvider] object. It will then return the telecommand
        /// and the according accepted token.
        pub fn retrieve_and_accept_next_packet(
            &mut self,
        ) -> Result<Option<AcceptedEcssTcAndToken>, PusPacketHandlingError> {
            match self.common.tc_receiver.recv_tc() {
                Ok(EcssTcAndToken {
                    tc_in_memory,
                    token,
                }) => {
                    if token.is_none() {
                        return Err(PusPacketHandlingError::InvalidVerificationToken);
                    }
                    let token = token.unwrap();
                    let accepted_token = VerificationToken::<TcStateAccepted>::try_from(token)
                        .map_err(|_| PusPacketHandlingError::InvalidVerificationToken)?;
                    Ok(Some(AcceptedEcssTcAndToken {
                        tc_in_memory,
                        token: accepted_token,
                    }))
                }
                Err(e) => match e {
                    TryRecvTmtcError::Tmtc(e) => Err(PusPacketHandlingError::TcPolling(e)),
                    TryRecvTmtcError::Empty => Ok(None),
                },
            }
        }

        pub fn verif_reporter(&self) -> &VerificationReporter {
            &self.common.verif_reporter
        }

        pub fn tc_in_mem_converter(&self) -> &TcInMemConverter {
            &self.tc_in_mem_converter
        }

        pub fn tc_in_mem_converter_mut(&mut self) -> &mut TcInMemConverter {
            &mut self.tc_in_mem_converter
        }
    }

    pub type PusServiceHelperDynWithMpsc<TcInMemConverter, VerificationReporter> = PusServiceHelper<
        MpscTcReceiver,
        TmAsVecSenderWithMpsc,
        TcInMemConverter,
        VerificationReporter,
    >;
    pub type PusServiceHelperDynWithBoundedMpsc<TcInMemConverter, VerificationReporter> =
        PusServiceHelper<
            MpscTcReceiver,
            TmAsVecSenderWithBoundedMpsc,
            TcInMemConverter,
            VerificationReporter,
        >;
    pub type PusServiceHelperStaticWithMpsc<TcInMemConverter, VerificationReporter> =
        PusServiceHelper<
            MpscTcReceiver,
            TmInSharedPoolSenderWithMpsc,
            TcInMemConverter,
            VerificationReporter,
        >;
    pub type PusServiceHelperStaticWithBoundedMpsc<TcInMemConverter, VerificationReporter> =
        PusServiceHelper<
            MpscTcReceiver,
            TmInSharedPoolSenderWithBoundedMpsc,
            TcInMemConverter,
            VerificationReporter,
        >;
}

pub(crate) fn source_buffer_large_enough(cap: usize, len: usize) -> Result<(), EcssTmtcError> {
    if len > cap {
        return Err(
            PusError::ByteConversion(ByteConversionError::ToSliceTooSmall {
                found: cap,
                expected: len,
            })
            .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use core::cell::RefCell;
    use std::sync::mpsc::TryRecvError;
    use std::sync::{mpsc, RwLock};

    use alloc::collections::VecDeque;
    use alloc::vec::Vec;
    use satrs_shared::res_code::ResultU16;
    use spacepackets::ecss::tc::{PusTcCreator, PusTcReader};
    use spacepackets::ecss::tm::{GenericPusTmSecondaryHeader, PusTmCreator, PusTmReader};
    use spacepackets::ecss::{PusPacket, WritablePusPacket};
    use spacepackets::CcsdsPacket;

    use crate::pool::{
        PoolProvider, SharedStaticMemoryPool, StaticMemoryPool, StaticPoolConfig, StoreAddr,
    };
    use crate::pus::verification::RequestId;
    use crate::tmtc::tm_helper::SharedTmPool;
    use crate::ComponentId;

    use super::verification::std_mod::{
        VerificationReporterWithSharedPoolMpscBoundedSender, VerificationReporterWithVecMpscSender,
    };
    use super::verification::tests::{SharedVerificationMap, TestVerificationReporter};
    use super::verification::{
        TcStateAccepted, VerificationReporterCfg, VerificationReporterWithSender,
        VerificationReportingProvider, VerificationToken,
    };
    use super::{
        EcssTcAndToken, EcssTcInSharedStoreConverter, EcssTcInVecConverter, GenericConversionError,
        GenericRoutingError, MpscTcReceiver, PusPacketHandlerResult, PusPacketHandlingError,
        PusServiceHelper, TcInMemory, TmAsVecSenderWithId, TmAsVecSenderWithMpsc,
        TmInSharedPoolSenderWithBoundedMpsc, TmInSharedPoolSenderWithId,
    };

    pub const TEST_APID: u16 = 0x101;

    #[derive(Debug, Eq, PartialEq, Clone)]
    pub(crate) struct CommonTmInfo {
        pub subservice: u8,
        pub apid: u16,
        pub msg_counter: u16,
        pub dest_id: u16,
        pub time_stamp: [u8; 7],
    }

    pub trait PusTestHarness {
        fn send_tc(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted>;
        fn read_next_tm(&mut self) -> PusTmReader<'_>;
        fn check_no_tm_available(&self) -> bool;
        fn check_next_verification_tm(&self, subservice: u8, expected_request_id: RequestId);
    }

    pub trait SimplePusPacketHandler {
        fn handle_one_tc(&mut self) -> Result<PusPacketHandlerResult, PusPacketHandlingError>;
    }

    impl CommonTmInfo {
        pub fn new_from_tm(tm: &PusTmCreator) -> Self {
            let mut time_stamp = [0; 7];
            time_stamp.clone_from_slice(&tm.timestamp()[0..7]);
            Self {
                subservice: PusPacket::subservice(tm),
                apid: tm.apid(),
                msg_counter: tm.msg_counter(),
                dest_id: tm.dest_id(),
                time_stamp,
            }
        }
    }

    /// Common fields for a PUS service test harness.
    pub struct PusServiceHandlerWithSharedStoreCommon {
        pus_buf: [u8; 2048],
        tm_buf: [u8; 2048],
        tc_pool: SharedStaticMemoryPool,
        tm_pool: SharedTmPool,
        tc_sender: mpsc::SyncSender<EcssTcAndToken>,
        tm_receiver: mpsc::Receiver<StoreAddr>,
        verification_handler: VerificationReporterWithSharedPoolMpscBoundedSender,
    }

    pub type PusServiceHelperStatic = PusServiceHelper<
        MpscTcReceiver,
        TmInSharedPoolSenderWithBoundedMpsc,
        EcssTcInSharedStoreConverter,
        VerificationReporterWithSharedPoolMpscBoundedSender,
    >;

    impl PusServiceHandlerWithSharedStoreCommon {
        /// This function generates the structure in addition to the PUS service handler
        /// [PusServiceHandler] which might be required for a specific PUS service handler.
        ///
        /// The PUS service handler is instantiated with a [EcssTcInStoreConverter].
        pub fn new() -> (Self, PusServiceHelperStatic) {
            let pool_cfg = StaticPoolConfig::new(alloc::vec![(16, 16), (8, 32), (4, 64)], false);
            let tc_pool = StaticMemoryPool::new(pool_cfg.clone());
            let tm_pool = StaticMemoryPool::new(pool_cfg);
            let shared_tc_pool = SharedStaticMemoryPool::new(RwLock::new(tc_pool));
            let shared_tm_pool = SharedTmPool::new(tm_pool);
            let (test_srv_tc_tx, test_srv_tc_rx) = mpsc::sync_channel(10);
            let (tm_tx, tm_rx) = mpsc::sync_channel(10);

            let verif_sender = TmInSharedPoolSenderWithBoundedMpsc::new(
                0,
                "verif_sender",
                shared_tm_pool.clone(),
                tm_tx.clone(),
            );
            let verif_cfg = VerificationReporterCfg::new(TEST_APID, 1, 2, 8).unwrap();
            let verification_handler =
                VerificationReporterWithSharedPoolMpscBoundedSender::new(&verif_cfg, verif_sender);
            let test_srv_tm_sender =
                TmInSharedPoolSenderWithId::new(0, "TEST_SENDER", shared_tm_pool.clone(), tm_tx);
            let test_srv_tc_receiver = MpscTcReceiver::new(0, "TEST_RECEIVER", test_srv_tc_rx);
            let in_store_converter =
                EcssTcInSharedStoreConverter::new(shared_tc_pool.clone(), 2048);
            (
                Self {
                    pus_buf: [0; 2048],
                    tm_buf: [0; 2048],
                    tc_pool: shared_tc_pool,
                    tm_pool: shared_tm_pool,
                    tc_sender: test_srv_tc_tx,
                    tm_receiver: tm_rx,
                    verification_handler: verification_handler.clone(),
                },
                PusServiceHelper::new(
                    test_srv_tc_receiver,
                    test_srv_tm_sender,
                    TEST_APID,
                    verification_handler,
                    in_store_converter,
                ),
            )
        }
        pub fn send_tc(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted> {
            let token = self.verification_handler.add_tc(tc);
            let token = self
                .verification_handler
                .acceptance_success(token, &[0; 7])
                .unwrap();
            let tc_size = tc.write_to_bytes(&mut self.pus_buf).unwrap();
            let mut tc_pool = self.tc_pool.write().unwrap();
            let addr = tc_pool.add(&self.pus_buf[..tc_size]).unwrap();
            drop(tc_pool);
            // Send accepted TC to test service handler.
            self.tc_sender
                .send(EcssTcAndToken::new(addr, token))
                .expect("sending tc failed");
            token
        }

        pub fn read_next_tm(&mut self) -> PusTmReader<'_> {
            let next_msg = self.tm_receiver.try_recv();
            assert!(next_msg.is_ok());
            let tm_addr = next_msg.unwrap();
            let tm_pool = self.tm_pool.0.read().unwrap();
            let tm_raw = tm_pool.read_as_vec(&tm_addr).unwrap();
            self.tm_buf[0..tm_raw.len()].copy_from_slice(&tm_raw);
            PusTmReader::new(&self.tm_buf, 7).unwrap().0
        }

        pub fn check_no_tm_available(&self) -> bool {
            let next_msg = self.tm_receiver.try_recv();
            if let TryRecvError::Empty = next_msg.unwrap_err() {
                return true;
            }
            false
        }

        pub fn check_next_verification_tm(&self, subservice: u8, expected_request_id: RequestId) {
            let next_msg = self.tm_receiver.try_recv();
            assert!(next_msg.is_ok());
            let tm_addr = next_msg.unwrap();
            let tm_pool = self.tm_pool.0.read().unwrap();
            let tm_raw = tm_pool.read_as_vec(&tm_addr).unwrap();
            let tm = PusTmReader::new(&tm_raw, 7).unwrap().0;
            assert_eq!(PusPacket::service(&tm), 1);
            assert_eq!(PusPacket::subservice(&tm), subservice);
            assert_eq!(tm.apid(), TEST_APID);
            let req_id =
                RequestId::from_bytes(tm.user_data()).expect("generating request ID failed");
            assert_eq!(req_id, expected_request_id);
        }
    }

    pub struct PusServiceHandlerWithVecCommon<VerificationReporter: VerificationReportingProvider> {
        current_tm: Option<alloc::vec::Vec<u8>>,
        tc_sender: mpsc::Sender<EcssTcAndToken>,
        tm_receiver: mpsc::Receiver<alloc::vec::Vec<u8>>,
        pub verification_handler: VerificationReporter,
    }
    pub type PusServiceHelperDynamic = PusServiceHelper<
        MpscTcReceiver,
        TmAsVecSenderWithMpsc,
        EcssTcInVecConverter,
        VerificationReporterWithVecMpscSender,
    >;

    impl PusServiceHandlerWithVecCommon<VerificationReporterWithVecMpscSender> {
        pub fn new_with_standard_verif_reporter() -> (Self, PusServiceHelperDynamic) {
            let (test_srv_tc_tx, test_srv_tc_rx) = mpsc::channel();
            let (tm_tx, tm_rx) = mpsc::channel();

            let verif_sender = TmAsVecSenderWithId::new(0, "verififcatio-sender", tm_tx.clone());
            let verif_cfg = VerificationReporterCfg::new(TEST_APID, 1, 2, 8).unwrap();
            let verification_handler =
                VerificationReporterWithSender::new(&verif_cfg, verif_sender);

            let test_srv_tm_sender = TmAsVecSenderWithId::new(0, "test-sender", tm_tx);
            let test_srv_tc_receiver = MpscTcReceiver::new(0, "test-receiver", test_srv_tc_rx);
            let in_store_converter = EcssTcInVecConverter::default();
            (
                Self {
                    current_tm: None,
                    tc_sender: test_srv_tc_tx,
                    tm_receiver: tm_rx,
                    verification_handler: verification_handler.clone(),
                },
                PusServiceHelper::new(
                    test_srv_tc_receiver,
                    test_srv_tm_sender,
                    TEST_APID,
                    verification_handler,
                    in_store_converter,
                ),
            )
        }
    }

    impl PusServiceHandlerWithVecCommon<TestVerificationReporter> {
        pub fn new_with_test_verif_sender() -> (
            Self,
            PusServiceHelper<
                MpscTcReceiver,
                TmAsVecSenderWithMpsc,
                EcssTcInVecConverter,
                TestVerificationReporter,
            >,
        ) {
            let (test_srv_tc_tx, test_srv_tc_rx) = mpsc::channel();
            let (tm_tx, tm_rx) = mpsc::channel();

            let test_srv_tm_sender = TmAsVecSenderWithId::new(0, "test-sender", tm_tx);
            let test_srv_tc_receiver = MpscTcReceiver::new(0, "test-receiver", test_srv_tc_rx);
            let in_store_converter = EcssTcInVecConverter::default();
            let shared_verif_map = SharedVerificationMap::default();
            let verification_handler = TestVerificationReporter::new(shared_verif_map);
            (
                Self {
                    current_tm: None,
                    tc_sender: test_srv_tc_tx,
                    tm_receiver: tm_rx,
                    verification_handler: verification_handler.clone(),
                },
                PusServiceHelper::new(
                    test_srv_tc_receiver,
                    test_srv_tm_sender,
                    TEST_APID,
                    verification_handler,
                    in_store_converter,
                ),
            )
        }
    }

    impl<VerificationReporter: VerificationReportingProvider>
        PusServiceHandlerWithVecCommon<VerificationReporter>
    {
        pub fn send_tc(&mut self, tc: &PusTcCreator) -> VerificationToken<TcStateAccepted> {
            let token = self.verification_handler.add_tc(tc);
            let token = self
                .verification_handler
                .acceptance_success(token, &[0; 7])
                .unwrap();
            // Send accepted TC to test service handler.
            self.tc_sender
                .send(EcssTcAndToken::new(
                    TcInMemory::Vec(tc.to_vec().expect("pus tc conversion to vec failed")),
                    token,
                ))
                .expect("sending tc failed");
            token
        }

        pub fn read_next_tm(&mut self) -> PusTmReader<'_> {
            let next_msg = self.tm_receiver.try_recv();
            assert!(next_msg.is_ok());
            self.current_tm = Some(next_msg.unwrap());
            PusTmReader::new(self.current_tm.as_ref().unwrap(), 7)
                .unwrap()
                .0
        }

        pub fn check_no_tm_available(&self) -> bool {
            let next_msg = self.tm_receiver.try_recv();
            if let TryRecvError::Empty = next_msg.unwrap_err() {
                return true;
            }
            false
        }

        pub fn check_next_verification_tm(&self, subservice: u8, expected_request_id: RequestId) {
            let next_msg = self.tm_receiver.try_recv();
            assert!(next_msg.is_ok());
            let next_msg = next_msg.unwrap();
            let tm = PusTmReader::new(next_msg.as_slice(), 7).unwrap().0;
            assert_eq!(PusPacket::service(&tm), 1);
            assert_eq!(PusPacket::subservice(&tm), subservice);
            assert_eq!(tm.apid(), TEST_APID);
            let req_id =
                RequestId::from_bytes(tm.user_data()).expect("generating request ID failed");
            assert_eq!(req_id, expected_request_id);
        }
    }

    pub const APP_DATA_TOO_SHORT: ResultU16 = ResultU16::new(1, 1);

    #[derive(Default)]
    pub struct TestConverter<const SERVICE: u8> {
        pub conversion_request: VecDeque<Vec<u8>>,
    }

    impl<const SERVICE: u8> TestConverter<SERVICE> {
        pub fn check_service(&self, tc: &PusTcReader) -> Result<(), PusPacketHandlingError> {
            if tc.service() != SERVICE {
                return Err(PusPacketHandlingError::RequestConversion(
                    GenericConversionError::WrongService(tc.service()),
                ));
            }
            Ok(())
        }

        pub fn is_empty(&self) {
            self.conversion_request.is_empty();
        }

        pub fn check_next_conversion(&mut self, tc: &PusTcCreator) {
            assert!(!self.conversion_request.is_empty());
            assert_eq!(
                self.conversion_request.pop_front().unwrap(),
                tc.to_vec().unwrap()
            );
        }
    }

    pub struct TestRouter<REQUEST> {
        pub routing_requests: RefCell<VecDeque<(ComponentId, REQUEST)>>,
        pub routing_errors: RefCell<VecDeque<(ComponentId, GenericRoutingError)>>,
        pub injected_routing_failure: RefCell<Option<GenericRoutingError>>,
    }

    impl<REQUEST> Default for TestRouter<REQUEST> {
        fn default() -> Self {
            Self {
                routing_requests: Default::default(),
                routing_errors: Default::default(),
                injected_routing_failure: Default::default(),
            }
        }
    }

    impl<REQUEST> TestRouter<REQUEST> {
        pub fn check_for_injected_error(&self) -> Result<(), GenericRoutingError> {
            if self.injected_routing_failure.borrow().is_some() {
                return Err(self.injected_routing_failure.borrow_mut().take().unwrap());
            }
            Ok(())
        }

        pub fn handle_error(
            &self,
            target_id: ComponentId,
            _token: VerificationToken<TcStateAccepted>,
            _tc: &PusTcReader,
            error: GenericRoutingError,
            _time_stamp: &[u8],
            _verif_reporter: &impl VerificationReportingProvider,
        ) {
            self.routing_errors
                .borrow_mut()
                .push_back((target_id, error));
        }

        pub fn no_routing_errors(&self) -> bool {
            self.routing_errors.borrow().is_empty()
        }

        pub fn retrieve_next_routing_error(&mut self) -> (ComponentId, GenericRoutingError) {
            if self.routing_errors.borrow().is_empty() {
                panic!("no routing request available");
            }
            self.routing_errors.borrow_mut().pop_front().unwrap()
        }

        pub fn inject_routing_error(&mut self, error: GenericRoutingError) {
            *self.injected_routing_failure.borrow_mut() = Some(error);
        }

        pub fn is_empty(&self) -> bool {
            self.routing_requests.borrow().is_empty()
        }

        pub fn retrieve_next_request(&mut self) -> (ComponentId, REQUEST) {
            if self.routing_requests.borrow().is_empty() {
                panic!("no routing request available");
            }
            self.routing_requests.borrow_mut().pop_front().unwrap()
        }
    }
}
