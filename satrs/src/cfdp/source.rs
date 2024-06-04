use spacepackets::cfdp::{pdu::FileDirectiveType, PduType};

use super::{
    filestore::VirtualFilestore, user::CfdpUser, LocalEntityConfig, PacketInfo, PacketTarget,
    PduSendProvider, RemoteEntityConfigProvider, UserFaultHookProvider,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TransactionStep {
    Idle = 0,
    TransactionStart = 1,
    SendingMetadata = 3,
    SendingFileData = 4,
    /// Re-transmitting missing packets in acknowledged mode6
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
}

pub struct SourceHandler<
    PduSender: PduSendProvider,
    UserFaultHook: UserFaultHookProvider,
    Vfs: VirtualFilestore,
    RemoteCfgTable: RemoteEntityConfigProvider,
> {
    local_cfg: LocalEntityConfig<UserFaultHook>,
    pdu_sender: PduSender,
    remote_cfg_table: RemoteCfgTable,
    vfs: Vfs,
    state_helper: StateHelper,
}

impl<
        PduSender: PduSendProvider,
        UserFaultHook: UserFaultHookProvider,
        Vfs: VirtualFilestore,
        RemoteCfgTable: RemoteEntityConfigProvider,
    > SourceHandler<PduSender, UserFaultHook, Vfs, RemoteCfgTable>
{
    pub fn new(
        cfg: LocalEntityConfig<UserFaultHook>,
        pdu_sender: PduSender,
        vfs: Vfs,
        remote_cfg_table: RemoteCfgTable,
    ) -> Self {
        Self {
            local_cfg: cfg,
            remote_cfg_table,
            vfs,
            pdu_sender,
            state_helper: Default::default(),
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

    fn fsm_busy(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<u32, SourceError> {
        Ok(0)
    }

    fn handle_finished_pdu(&mut self) {}

    fn handle_nak_pdu(&mut self) {}

    fn handle_keep_alive_pdu(&mut self) {}
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use spacepackets::util::UnsignedByteFieldU16;

    use super::*;
    use crate::cfdp::{
        filestore::NativeFilestore,
        tests::{basic_remote_cfg_table, TestCfdpSender, TestFaultHandler},
        FaultHandler, IndicationConfig, StdRemoteEntityConfigProvider,
    };

    const LOCAL_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(1);
    const REMOTE_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(2);

    type TestSourceHandler = SourceHandler<
        TestCfdpSender,
        TestFaultHandler,
        NativeFilestore,
        StdRemoteEntityConfigProvider,
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
        SourceHandler::new(
            local_entity_cfg,
            test_packet_sender,
            NativeFilestore::default(),
            basic_remote_cfg_table(),
            // TestCheckTimerCreator::new(check_timer_expired),
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
