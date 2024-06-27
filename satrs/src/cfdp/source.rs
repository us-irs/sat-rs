use spacepackets::{
    cfdp::{pdu::FileDirectiveType, PduType},
    util::UnsignedByteField,
    ByteConversionError,
};

use crate::seq_count::SequenceCountProvider;

use super::{
    filestore::VirtualFilestore,
    request::{ReadablePutRequest, StaticPutRequestCacher},
    user::CfdpUser,
    LocalEntityConfig, PacketInfo, PacketTarget, PduSendProvider, RemoteEntityConfig,
    RemoteEntityConfigProvider, TransactionId, UserFaultHookProvider,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TransactionStep {
    Idle = 0,
    TransactionStart = 1,
    SendingMetadata = 3,
    SendingFileData = 4,
    /// Re-transmitting missing packets in acknowledged mode
    Retransmitting = 5,
    SendingEof = 6,
    WaitingForEofAck = 7,
    WaitingForFinished = 8,
    SendingAckOfFinished = 9,
    NoticeOfCompletion = 10,
}

pub struct FileParams {
    pub progress: usize,
    pub segment_len: usize,
    pub crc32: Option<[u8; 4]>,
    pub metadata_only: bool,
    pub file_size: usize,
    pub no_eof: bool,
}

pub struct StateHelper {
    state: super::State,
    step: TransactionStep,
    num_packets_ready: u32,
}

#[derive(Debug, Copy, Clone, derive_new::new)]
pub struct TransferState {
    transaction_id: TransactionId,
    remote_cfg: RemoteEntityConfig,
    transmission_mode: super::TransmissionMode,
    closure_requested: bool,
}

impl Default for StateHelper {
    fn default() -> Self {
        Self {
            state: super::State::Idle,
            step: TransactionStep::Idle,
            num_packets_ready: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("can not process packet type {pdu_type:?} with directive type {directive_type:?}")]
    CantProcessPacketType {
        pdu_type: PduType,
        directive_type: Option<FileDirectiveType>,
    },
    #[error("unexpected file data PDU")]
    UnexpectedFileDataPdu,
    #[error("source handler is already busy with put request")]
    PutRequestAlreadyActive,
    #[error("error caching put request")]
    PutRequestCaching(ByteConversionError),
}

#[derive(Debug, thiserror::Error)]
pub enum PutRequestError {
    #[error("error caching put request: {0}")]
    Storage(#[from] ByteConversionError),
    #[error("already busy with put request")]
    AlreadyBusy,
}

pub struct SourceHandler<
    PduSender: PduSendProvider,
    UserFaultHook: UserFaultHookProvider,
    Vfs: VirtualFilestore,
    RemoteCfgTable: RemoteEntityConfigProvider,
    SeqCountProvider: SequenceCountProvider,
> {
    local_cfg: LocalEntityConfig<UserFaultHook>,
    pdu_sender: PduSender,
    put_request_cacher: StaticPutRequestCacher,
    remote_cfg_table: RemoteCfgTable,
    vfs: Vfs,
    state_helper: StateHelper,
    tstate: Option<TransferState>,
    seq_count_provider: SeqCountProvider,
}

impl<
        PduSender: PduSendProvider,
        UserFaultHook: UserFaultHookProvider,
        Vfs: VirtualFilestore,
        RemoteCfgTable: RemoteEntityConfigProvider,
        SeqCountProvider: SequenceCountProvider,
    > SourceHandler<PduSender, UserFaultHook, Vfs, RemoteCfgTable, SeqCountProvider>
{
    pub fn new(
        cfg: LocalEntityConfig<UserFaultHook>,
        pdu_sender: PduSender,
        vfs: Vfs,
        put_request_cacher: StaticPutRequestCacher,
        remote_cfg_table: RemoteCfgTable,
        seq_count_provider: SeqCountProvider,
    ) -> Self {
        Self {
            local_cfg: cfg,
            remote_cfg_table,
            pdu_sender,
            vfs,
            put_request_cacher,
            state_helper: Default::default(),
            tstate: Default::default(),
            seq_count_provider,
        }
    }

    /// This is the core function to drive the source handler. It is also used to insert
    /// packets into the source handler.
    ///
    /// The state machine should either be called if a packet with the appropriate destination ID
    /// is received, or periodically in IDLE periods to perform all CFDP related tasks, for example
    /// checking for timeouts or missed file segments.
    ///
    /// The function returns the number of sent PDU packets on success.
    pub fn state_machine(
        &mut self,
        cfdp_user: &mut impl CfdpUser,
        packet_to_insert: Option<&PacketInfo>,
    ) -> Result<u32, SourceError> {
        if let Some(packet) = packet_to_insert {
            self.insert_packet(cfdp_user, packet)?;
        }
        match self.state_helper.state {
            super::State::Idle => todo!(),
            super::State::Busy => self.fsm_busy(cfdp_user),
            super::State::Suspended => todo!(),
        }
    }

    fn insert_packet(
        &mut self,
        cfdp_user: &mut impl CfdpUser,
        packet_info: &PacketInfo,
    ) -> Result<(), SourceError> {
        if packet_info.target() != PacketTarget::SourceEntity {
            // Unwrap is okay here, a PacketInfo for a file data PDU should always have the
            // destination as the target.
            return Err(SourceError::CantProcessPacketType {
                pdu_type: packet_info.pdu_type(),
                directive_type: packet_info.pdu_directive(),
            });
        }
        if packet_info.pdu_type() == PduType::FileData {
            // The [PacketInfo] API should ensure that file data PDUs can not be passed
            // into a source entity, so this should never happen.
            return Err(SourceError::UnexpectedFileDataPdu);
        }
        // Unwrap is okay here, the [PacketInfo] API should ensure that the directive type is
        // always a valid value.
        match packet_info
            .pdu_directive()
            .expect("PDU directive type unexpectedly not set")
        {
            FileDirectiveType::FinishedPdu => self.handle_finished_pdu(),
            FileDirectiveType::NakPdu => self.handle_nak_pdu(),
            FileDirectiveType::KeepAlivePdu => self.handle_keep_alive_pdu(),
            FileDirectiveType::AckPdu => todo!("acknowledged mode not implemented yet"),
            FileDirectiveType::EofPdu
            | FileDirectiveType::PromptPdu
            | FileDirectiveType::MetadataPdu => {
                return Err(SourceError::CantProcessPacketType {
                    pdu_type: packet_info.pdu_type(),
                    directive_type: packet_info.pdu_directive(),
                });
            }
        }
        Ok(())
    }

    pub fn put_request(
        &mut self,
        put_request: &impl ReadablePutRequest,
    ) -> Result<(), PutRequestError> {
        if self.state_helper.state != super::State::Idle {
            return Err(PutRequestError::AlreadyBusy);
        }
        self.put_request_cacher.set(put_request)?;
        self.state_helper.state = super::State::Busy;
        let source_file = self.put_request_cacher.source_file().unwrap();
        if !self.vfs.exists(source_file) {
            // TODO: Specific error.
        }
        let remote_cfg = self.remote_cfg_table.get(
            self.put_request_cacher
                .static_fields
                .destination_id
                .value_const(),
        );
        if remote_cfg.is_none() {
            // TODO: Specific error.
        }
        let remote_cfg = remote_cfg.unwrap();
        self.state_helper.num_packets_ready = 0;
        //self.tstate.remote_cfg = Some(*remote_cfg);
        let transmission_mode = if self.put_request_cacher.static_fields.trans_mode.is_some() {
            self.put_request_cacher.static_fields.trans_mode.unwrap()
        } else {
            remote_cfg.default_transmission_mode
        };
        let closure_requested = if self
            .put_request_cacher
            .static_fields
            .closure_requested
            .is_some()
        {
            self.put_request_cacher
                .static_fields
                .closure_requested
                .unwrap()
        } else {
            remote_cfg.closure_requested_by_default
        };
        self.tstate = Some(TransferState::new(
            TransactionId::new(
                self.put_request_cacher.static_fields.destination_id,
                UnsignedByteField::new(
                    SeqCountProvider::MAX_BIT_WIDTH / 8,
                    self.seq_count_provider.get_and_increment().into(),
                ),
            ),
            *remote_cfg,
            transmission_mode,
            closure_requested,
        ));
        Ok(())
    }

    pub fn transmission_mode(&self) {}

    fn fsm_busy(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<u32, SourceError> {
        Ok(0)
    }

    fn handle_finished_pdu(&mut self) {}

    fn handle_nak_pdu(&mut self) {}

    fn handle_keep_alive_pdu(&mut self) {}
}

#[cfg(test)]
mod tests {
    use spacepackets::util::UnsignedByteFieldU16;

    use super::*;
    use crate::{
        cfdp::{
            filestore::NativeFilestore,
            tests::{basic_remote_cfg_table, TestCfdpSender, TestFaultHandler},
            FaultHandler, IndicationConfig, StdRemoteEntityConfigProvider,
        },
        seq_count::SeqCountProviderSimple,
    };

    const LOCAL_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(1);
    const REMOTE_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(2);

    type TestSourceHandler = SourceHandler<
        TestCfdpSender,
        TestFaultHandler,
        NativeFilestore,
        StdRemoteEntityConfigProvider,
        SeqCountProviderSimple<u16>,
    >;

    fn default_source_handler(
        test_fault_handler: TestFaultHandler,
        test_packet_sender: TestCfdpSender,
    ) -> TestSourceHandler {
        let local_entity_cfg = LocalEntityConfig {
            id: REMOTE_ID.into(),
            indication_cfg: IndicationConfig::default(),
            fault_handler: FaultHandler::new(test_fault_handler),
        };
        let static_put_request_cacher = StaticPutRequestCacher::new(1024);
        SourceHandler::new(
            local_entity_cfg,
            test_packet_sender,
            NativeFilestore::default(),
            static_put_request_cacher,
            basic_remote_cfg_table(),
            SeqCountProviderSimple::default(),
        )
    }

    #[test]
    fn test_basic() {
        let fault_handler = TestFaultHandler::default();
        let test_sender = TestCfdpSender::default();
        let source_handler = default_source_handler(fault_handler, test_sender);
        // assert!(dest_handler.transmission_mode().is_none());
        // assert!(fault_handler.all_queues_empty());
    }
}
