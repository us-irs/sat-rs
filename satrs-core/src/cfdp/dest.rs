use crate::cfdp::user::TransactionFinishedParams;
use core::str::{from_utf8, Utf8Error};
use std::path::{Path, PathBuf};

use super::{
    filestore::{FilestoreError, VirtualFilestore},
    user::{CfdpUser, FileSegmentRecvdParams, MetadataReceivedParams},
    CheckTimer, CheckTimerCreator, EntityType, LocalEntityConfig, PacketInfo, PacketTarget,
    RemoteEntityConfig, RemoteEntityConfigProvider, State, TransactionId, TransactionStep,
};
use alloc::boxed::Box;
use smallvec::SmallVec;
use spacepackets::{
    cfdp::{
        pdu::{
            eof::EofPdu,
            file_data::FileDataPdu,
            finished::{DeliveryCode, FileStatus, FinishedPduCreator},
            metadata::{MetadataGenericParams, MetadataPduReader},
            CfdpPdu, CommonPduConfig, FileDirectiveType, PduError, PduHeader, WritablePduPacket,
        },
        tlv::{msg_to_user::MsgToUserTlv, EntityIdTlv, GenericTlv, TlvType},
        ConditionCode, FaultHandlerCode, PduType, TransmissionMode,
    },
    util::UnsignedByteField,
};
use thiserror::Error;

#[derive(Debug, Default)]
struct PacketsToSendContext {
    packet_available: bool,
    directive: Option<FileDirectiveType>,
}

#[derive(Debug)]
struct FileProperties {
    src_file_name: [u8; u8::MAX as usize],
    src_file_name_len: usize,
    dest_file_name: [u8; u8::MAX as usize],
    dest_file_name_len: usize,
    dest_path_buf: PathBuf,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum CompletionDisposition {
    Completed = 0,
    Cancelled = 1,
}

#[derive(Debug)]
struct TransferState {
    transaction_id: Option<TransactionId>,
    metadata_params: MetadataGenericParams,
    progress: u64,
    condition_code: ConditionCode,
    delivery_code: DeliveryCode,
    file_status: FileStatus,
    completion_disposition: CompletionDisposition,
    checksum: u32,
    current_check_count: u32,
    current_check_timer: Option<Box<dyn CheckTimer>>,
}

impl Default for TransferState {
    fn default() -> Self {
        Self {
            transaction_id: None,
            metadata_params: Default::default(),
            progress: Default::default(),
            condition_code: ConditionCode::NoError,
            delivery_code: DeliveryCode::Incomplete,
            file_status: FileStatus::Unreported,
            completion_disposition: CompletionDisposition::Completed,
            checksum: 0,
            current_check_count: 0,
            current_check_timer: None,
        }
    }
}

#[derive(Debug)]
struct TransactionParams {
    tstate: TransferState,
    pdu_conf: CommonPduConfig,
    file_properties: FileProperties,
    cksum_buf: [u8; 1024],
    msgs_to_user_size: usize,
    msgs_to_user_buf: [u8; 1024],
    remote_cfg: Option<RemoteEntityConfig>,
}

impl TransactionParams {
    fn transmission_mode(&self) -> TransmissionMode {
        self.pdu_conf.trans_mode
    }
}

impl Default for FileProperties {
    fn default() -> Self {
        Self {
            src_file_name: [0; u8::MAX as usize],
            src_file_name_len: Default::default(),
            dest_file_name: [0; u8::MAX as usize],
            dest_file_name_len: Default::default(),
            dest_path_buf: Default::default(),
        }
    }
}

impl TransactionParams {
    fn file_size(&self) -> u64 {
        self.tstate.metadata_params.file_size
    }

    fn metadata_params(&self) -> &MetadataGenericParams {
        &self.tstate.metadata_params
    }
}

impl Default for TransactionParams {
    fn default() -> Self {
        Self {
            pdu_conf: Default::default(),
            cksum_buf: [0; 1024],
            msgs_to_user_size: 0,
            msgs_to_user_buf: [0; 1024],
            tstate: Default::default(),
            file_properties: Default::default(),
            remote_cfg: None,
        }
    }
}

impl TransactionParams {
    fn reset(&mut self) {
        self.tstate.condition_code = ConditionCode::NoError;
        self.tstate.delivery_code = DeliveryCode::Incomplete;
        self.tstate.file_status = FileStatus::Unreported;
    }
}

#[derive(Debug, Error)]
pub enum DestError {
    /// File directive expected, but none specified
    #[error("expected file directive")]
    DirectiveExpected,
    #[error("can not process packet type {0:?}")]
    CantProcessPacketType(FileDirectiveType),
    #[error("can not process file data PDUs in current state")]
    WrongStateForFileDataAndEof,
    // Received new metadata PDU while being already being busy with a file transfer.
    #[error("busy with transfer")]
    RecvdMetadataButIsBusy,
    #[error("empty source file field")]
    EmptySrcFileField,
    #[error("empty dest file field")]
    EmptyDestFileField,
    #[error("packets to be sent are still left")]
    PacketToSendLeft,
    #[error("pdu error {0}")]
    Pdu(#[from] PduError),
    #[error("io error {0}")]
    Io(#[from] std::io::Error),
    #[error("file store error {0}")]
    Filestore(#[from] FilestoreError),
    #[error("path conversion error {0}")]
    PathConversion(#[from] Utf8Error),
    #[error("error building dest path from source file name and dest folder")]
    PathConcat,
    #[error("no remote entity configuration found for {0:?}")]
    NoRemoteCfgFound(UnsignedByteField),
}

/// This is the primary CFDP destination handler. It models the CFDP destination entity, which is
/// primarily responsible for receiving files sent from another CFDP entity. It performs the
/// reception side of File Copy Operations.

/// The following core functions are the primary interface for interacting with the destination
/// handler:

/// 1. [DestinationHandler::state_machine] - Can be used to insert packets into the destination
///    handler and/or advance the state machine. Advancing the state machine might generate new
///    packets to be sent to the remote entity. Please note that the destination handler can also
///    only process Metadata, EOF and Prompt PDUs in addition to ACK PDUs where the acknowledged
///    PDU is the Finished PDU.
/// 2. [DestinationHandler::get_next_packet] - Retrieve next packet to be sent back to the remote
///    CFDP source entity ID.

/// A new file transfer (Metadata PDU reception) is only be accepted if the handler is in the
/// IDLE state. Furthermore, packet insertion is not allowed until all packets to send were
/// retrieved after a state machine call.
pub struct DestinationHandler {
    local_cfg: LocalEntityConfig,
    step: TransactionStep,
    state: State,
    tparams: TransactionParams,
    packets_to_send_ctx: PacketsToSendContext,
    vfs: Box<dyn VirtualFilestore>,
    remote_cfg_table: Box<dyn RemoteEntityConfigProvider>,
    check_timer_creator: Box<dyn CheckTimerCreator>,
}

impl DestinationHandler {
    pub fn new(
        local_cfg: LocalEntityConfig,
        vfs: Box<dyn VirtualFilestore>,
        remote_cfg_table: Box<dyn RemoteEntityConfigProvider>,
        check_timer_creator: Box<dyn CheckTimerCreator>,
    ) -> Self {
        Self {
            local_cfg,
            step: TransactionStep::Idle,
            state: State::Idle,
            tparams: Default::default(),
            packets_to_send_ctx: Default::default(),
            vfs,
            remote_cfg_table,
            check_timer_creator,
        }
    }

    /// This is the core function to drive the destination handler. It is also used to insert
    /// packets into the destination handler.
    ///
    /// Please note that this function will fail if there are still packets which need to be
    /// retrieved with [Self::get_next_packet]. After each state machine call, the user has to
    /// retrieve all packets before calling the state machine again. The state machine should
    /// either be called if a packet with the appropriate destination ID is received, or
    /// periodically in IDLE periods to perform all CFDP related tasks, for example checking for
    /// timeouts or missed file segments.
    pub fn state_machine(
        &mut self,
        cfdp_user: &mut impl CfdpUser,
        packet_to_insert: Option<&PacketInfo>,
    ) -> Result<(), DestError> {
        if self.packet_to_send_ready() {
            return Err(DestError::PacketToSendLeft);
        }
        if let Some(packet) = packet_to_insert {
            self.insert_packet(cfdp_user, packet)?;
        }
        match self.state {
            State::Idle => todo!(),
            State::Busy => self.fsm_busy(cfdp_user),
            State::Suspended => todo!(),
        }
    }

    /// Returns [None] if the state machine is IDLE, and the transmission mode of the current
    /// request otherwise.
    pub fn transmission_mode(&self) -> Option<TransmissionMode> {
        if self.state == State::Idle {
            return None;
        }
        Some(self.tparams.transmission_mode())
    }

    pub fn transaction_id(&self) -> Option<TransactionId> {
        self.tstate().transaction_id
    }

    pub fn packet_to_send_ready(&self) -> bool {
        self.packets_to_send_ctx.packet_available
    }

    pub fn get_next_packet(
        &self,
        buf: &mut [u8],
    ) -> Result<Option<(FileDirectiveType, usize)>, DestError> {
        if !self.packet_to_send_ready() {
            return Ok(None);
        }
        let directive = self.packets_to_send_ctx.directive.unwrap();
        let tstate = self.tstate();
        let written_size = match directive {
            FileDirectiveType::FinishedPdu => {
                let pdu_header = PduHeader::new_no_file_data(self.tparams.pdu_conf, 0);
                let finished_pdu = if tstate.condition_code == ConditionCode::NoError
                    || tstate.condition_code == ConditionCode::UnsupportedChecksumType
                {
                    FinishedPduCreator::new_default(
                        pdu_header,
                        tstate.delivery_code,
                        tstate.file_status,
                    )
                } else {
                    // TODO: Are there cases where this ID is actually the source entity ID?
                    let entity_id = EntityIdTlv::new(self.local_cfg.id);
                    FinishedPduCreator::new_with_error(
                        pdu_header,
                        tstate.condition_code,
                        tstate.delivery_code,
                        tstate.file_status,
                        entity_id,
                    )
                };
                finished_pdu.write_to_bytes(buf)?
            }
            FileDirectiveType::AckPdu => todo!(),
            FileDirectiveType::NakPdu => todo!(),
            FileDirectiveType::KeepAlivePdu => todo!(),
            _ => {
                // This should never happen and is considered an internal impl error
                panic!("invalid file directive {directive:?} for dest handler send packet");
            }
        };
        Ok(Some((directive, written_size)))
    }

    fn insert_packet(
        &mut self,
        cfdp_user: &mut impl CfdpUser,
        packet_info: &PacketInfo,
    ) -> Result<(), DestError> {
        if packet_info.target() != PacketTarget::DestEntity {
            // Unwrap is okay here, a PacketInfo for a file data PDU should always have the
            // destination as the target.
            return Err(DestError::CantProcessPacketType(
                packet_info.pdu_directive().unwrap(),
            ));
        }
        match packet_info.pdu_type {
            PduType::FileDirective => {
                if packet_info.pdu_directive.is_none() {
                    return Err(DestError::DirectiveExpected);
                }
                self.handle_file_directive(
                    cfdp_user,
                    packet_info.pdu_directive.unwrap(),
                    packet_info.raw_packet,
                )
            }
            PduType::FileData => self.handle_file_data(cfdp_user, packet_info.raw_packet),
        }
    }

    fn handle_file_directive(
        &mut self,
        cfdp_user: &mut impl CfdpUser,
        pdu_directive: FileDirectiveType,
        raw_packet: &[u8],
    ) -> Result<(), DestError> {
        match pdu_directive {
            FileDirectiveType::EofPdu => self.handle_eof_pdu(cfdp_user, raw_packet)?,
            FileDirectiveType::FinishedPdu
            | FileDirectiveType::NakPdu
            | FileDirectiveType::KeepAlivePdu => {
                return Err(DestError::CantProcessPacketType(pdu_directive));
            }
            FileDirectiveType::AckPdu => {
                todo!(
                "check whether ACK pdu handling is applicable by checking the acked directive field"
                )
            }
            FileDirectiveType::MetadataPdu => self.handle_metadata_pdu(raw_packet)?,
            FileDirectiveType::PromptPdu => self.handle_prompt_pdu(raw_packet)?,
        };
        Ok(())
    }

    fn handle_metadata_pdu(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
        if self.state != State::Idle {
            return Err(DestError::RecvdMetadataButIsBusy);
        }
        let metadata_pdu = MetadataPduReader::from_bytes(raw_packet)?;
        self.tparams.reset();
        self.tparams.tstate.metadata_params = *metadata_pdu.metadata_params();
        let remote_cfg = self
            .remote_cfg_table
            .get_remote_config(metadata_pdu.source_id().value());
        if remote_cfg.is_none() {
            return Err(DestError::NoRemoteCfgFound(metadata_pdu.dest_id()));
        }
        self.tparams.remote_cfg = Some(*remote_cfg.unwrap());

        // TODO: Support for metadata only PDUs.
        let src_name = metadata_pdu.src_file_name();
        if src_name.is_empty() {
            return Err(DestError::EmptySrcFileField);
        }
        self.tparams.file_properties.src_file_name[..src_name.len_value()]
            .copy_from_slice(src_name.value());
        self.tparams.file_properties.src_file_name_len = src_name.len_value();
        let dest_name = metadata_pdu.dest_file_name();
        if dest_name.is_empty() {
            return Err(DestError::EmptyDestFileField);
        }
        self.tparams.file_properties.dest_file_name[..dest_name.len_value()]
            .copy_from_slice(dest_name.value());
        self.tparams.file_properties.dest_file_name_len = dest_name.len_value();
        self.tparams.pdu_conf = *metadata_pdu.pdu_header().common_pdu_conf();
        self.tparams.msgs_to_user_size = 0;
        if !metadata_pdu.options().is_empty() {
            for option_tlv in metadata_pdu.options_iter().unwrap() {
                if option_tlv.is_standard_tlv()
                    && option_tlv.tlv_type().unwrap() == TlvType::MsgToUser
                {
                    self.tparams
                        .msgs_to_user_buf
                        .copy_from_slice(option_tlv.raw_data().unwrap());
                    self.tparams.msgs_to_user_size += option_tlv.len_full();
                }
            }
        }
        self.state = State::Busy;
        self.step = TransactionStep::TransactionStart;
        Ok(())
    }

    fn handle_file_data(
        &mut self,
        user: &mut impl CfdpUser,
        raw_packet: &[u8],
    ) -> Result<(), DestError> {
        if self.state == State::Idle
            || (self.step != TransactionStep::ReceivingFileDataPdus
                && self.step != TransactionStep::ReceivingFileDataPdusWithCheckLimitHandling)
        {
            return Err(DestError::WrongStateForFileDataAndEof);
        }
        let fd_pdu = FileDataPdu::from_bytes(raw_packet)?;
        if self.local_cfg.indication_cfg.file_segment_recv {
            user.file_segment_recvd_indication(&FileSegmentRecvdParams {
                id: self.tstate().transaction_id.unwrap(),
                offset: fd_pdu.offset(),
                length: fd_pdu.file_data().len(),
                segment_metadata: fd_pdu.segment_metadata(),
            });
        }
        if let Err(e) = self.vfs.write_data(
            self.tparams.file_properties.dest_path_buf.to_str().unwrap(),
            fd_pdu.offset(),
            fd_pdu.file_data(),
        ) {
            self.declare_fault(ConditionCode::FilestoreRejection);
            return Err(e.into());
        }
        self.tstate_mut().progress += fd_pdu.file_data().len() as u64;
        Ok(())
    }

    fn handle_eof_pdu(
        &mut self,
        cfdp_user: &mut impl CfdpUser,
        raw_packet: &[u8],
    ) -> Result<(), DestError> {
        if self.state == State::Idle || self.step != TransactionStep::ReceivingFileDataPdus {
            return Err(DestError::WrongStateForFileDataAndEof);
        }
        let eof_pdu = EofPdu::from_bytes(raw_packet)?;
        if self.local_cfg.indication_cfg.eof_recv {
            // Unwrap is okay here, application logic ensures that transaction ID is valid here.
            cfdp_user.eof_recvd_indication(self.tparams.tstate.transaction_id.as_ref().unwrap());
        }
        let regular_transfer_finish = if eof_pdu.condition_code() == ConditionCode::NoError {
            self.handle_no_error_eof_pdu(&eof_pdu)?
        } else {
            todo!("implement cancel request handling");
        };
        if regular_transfer_finish {
            self.file_transfer_complete_transition();
        }
        Ok(())
    }

    /// Returns whether the transfer can be completed regularly.
    fn handle_no_error_eof_pdu(&mut self, eof_pdu: &EofPdu) -> Result<bool, DestError> {
        // CFDP 4.6.1.2.9: Declare file size error if progress exceeds file size
        if self.tparams.tstate.progress > eof_pdu.file_size()
            && self.declare_fault(ConditionCode::FileSizeError) != FaultHandlerCode::IgnoreError
        {
            return Ok(false);
        } else if (self.tparams.tstate.progress < eof_pdu.file_size())
            && self.tparams.transmission_mode() == TransmissionMode::Acknowledged
        {
            // CFDP 4.6.4.3.1: The end offset of the last received file segment and the file
            // size as stated in the EOF PDU is not the same, so we need to add that segment to
            // the lost segments for the deferred lost segment detection procedure.
            // TODO: Proper lost segment handling.
            // self._params.acked_params.lost_seg_tracker.add_lost_segment(
            //  (self._params.fp.progress, self._params.fp.file_size_eof)
            // )
        }

        self.tparams.tstate.checksum = eof_pdu.file_checksum();
        if self.tparams.transmission_mode() == TransmissionMode::Unacknowledged
            && !self.checksum_verify(self.tparams.tstate.checksum)
        {
            if self.declare_fault(ConditionCode::FileChecksumFailure)
                != FaultHandlerCode::IgnoreError
            {
                return Ok(false);
            }
            self.start_check_limit_handling();
            return Ok(false);
        }
        Ok(true)
    }

    fn file_transfer_complete_transition(&mut self) {
        if self.tparams.transmission_mode() == TransmissionMode::Unacknowledged {
            self.step = TransactionStep::TransferCompletion;
        } else {
            // TODO: Prepare ACK PDU somehow.
            self.step = TransactionStep::SendingAckPdu;
        }
    }

    fn checksum_verify(&mut self, checksum: u32) -> bool {
        match self.vfs.checksum_verify(
            self.tparams.file_properties.dest_path_buf.to_str().unwrap(),
            self.tparams.metadata_params().checksum_type,
            checksum,
            &mut self.tparams.cksum_buf,
        ) {
            Ok(checksum_success) => checksum_success,
            Err(e) => match e {
                FilestoreError::ChecksumTypeNotImplemented(_) => {
                    self.declare_fault(ConditionCode::UnsupportedChecksumType);
                    // For this case, the applicable algorithm shall the the null checksum, which
                    // is always succesful.
                    true
                }
                _ => {
                    self.declare_fault(ConditionCode::FilestoreRejection);
                    // Treat this equivalent to a failed checksum procedure.
                    false
                }
            },
        }
    }

    fn start_check_limit_handling(&mut self) {
        self.step = TransactionStep::ReceivingFileDataPdusWithCheckLimitHandling;
        self.tparams.tstate.current_check_timer =
            Some(self.check_timer_creator.get_check_timer_provider(
                &self.local_cfg.id,
                &self.tparams.remote_cfg.unwrap().entity_id,
                EntityType::Receiving,
            ));
        self.tparams.tstate.current_check_count = 0;
    }

    fn check_limit_handling(&mut self) {
        if self.tparams.tstate.current_check_timer.is_none() {
            return;
        }
        let check_timer = self.tparams.tstate.current_check_timer.as_ref().unwrap();
        if check_timer.has_expired() {
            if self.checksum_verify(self.tparams.tstate.checksum) {
                self.file_transfer_complete_transition();
                return;
            }
            if self.tparams.tstate.current_check_count + 1
                >= self.tparams.remote_cfg.unwrap().check_limit
            {
                self.declare_fault(ConditionCode::CheckLimitReached);
            } else {
                self.tparams.tstate.current_check_count += 1;
                self.tparams
                    .tstate
                    .current_check_timer
                    .as_mut()
                    .unwrap()
                    .reset();
            }
        }
    }

    pub fn handle_prompt_pdu(&mut self, _raw_packet: &[u8]) -> Result<(), DestError> {
        todo!();
    }

    fn fsm_busy(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<(), DestError> {
        if self.step == TransactionStep::TransactionStart {
            self.transaction_start(cfdp_user)?;
        }
        if self.step == TransactionStep::ReceivingFileDataPdusWithCheckLimitHandling {
            self.check_limit_handling();
        }
        if self.step == TransactionStep::TransferCompletion {
            self.transfer_completion(cfdp_user)?;
        }
        if self.step == TransactionStep::SendingAckPdu {
            todo!("no support for acknowledged mode yet");
        }
        if self.step == TransactionStep::SendingFinishedPdu {
            self.reset();
        }
        Ok(())
    }

    /// Get the step, which denotes the exact step of a pending CFDP transaction when applicable.
    pub fn step(&self) -> TransactionStep {
        self.step
    }

    /// Get the step, which denotes whether the CFDP handler is active, and which CFDP class
    /// is used if it is active.
    pub fn state(&self) -> State {
        self.state
    }

    fn transaction_start(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<(), DestError> {
        let dest_name = from_utf8(
            &self.tparams.file_properties.dest_file_name
                [..self.tparams.file_properties.dest_file_name_len],
        )?;
        let dest_path = Path::new(dest_name);
        self.tparams.file_properties.dest_path_buf = dest_path.to_path_buf();
        let source_id = self.tparams.pdu_conf.source_id();
        let id = TransactionId::new(source_id, self.tparams.pdu_conf.transaction_seq_num);
        let src_name = from_utf8(
            &self.tparams.file_properties.src_file_name
                [0..self.tparams.file_properties.src_file_name_len],
        )?;
        let mut msgs_to_user = SmallVec::<[MsgToUserTlv<'_>; 16]>::new();
        let mut num_msgs_to_user = 0;
        if self.tparams.msgs_to_user_size > 0 {
            let mut index = 0;
            while index < self.tparams.msgs_to_user_size {
                // This should never panic as the validity of the options was checked beforehand.
                let msgs_to_user_tlv =
                    MsgToUserTlv::from_bytes(&self.tparams.msgs_to_user_buf[index..])
                        .expect("message to user creation failed unexpectedly");
                msgs_to_user.push(msgs_to_user_tlv);
                index += msgs_to_user_tlv.len_full();
                num_msgs_to_user += 1;
            }
        }
        let metadata_recvd_params = MetadataReceivedParams {
            id,
            source_id,
            file_size: self.tparams.file_size(),
            src_file_name: src_name,
            dest_file_name: dest_name,
            msgs_to_user: &msgs_to_user[..num_msgs_to_user],
        };
        self.tparams.tstate.transaction_id = Some(id);
        cfdp_user.metadata_recvd_indication(&metadata_recvd_params);

        // TODO: This is the only remaining function which uses std.. the easiest way would
        // probably be to use a static pre-allocated dest path buffer to store any concatenated
        // paths.
        if dest_path.exists() && self.vfs.is_dir(dest_path.to_str().unwrap()) {
            // Create new destination path by concatenating the last part of the source source
            // name and the destination folder. For example, for a source file of /tmp/hello.txt
            // and a destination name of /home/test, the resulting file name should be
            // /home/test/hello.txt
            let source_path = Path::new(from_utf8(
                &self.tparams.file_properties.src_file_name
                    [..self.tparams.file_properties.src_file_name_len],
            )?);
            let source_name = source_path.file_name();
            if source_name.is_none() {
                return Err(DestError::PathConcat);
            }
            let source_name = source_name.unwrap();
            self.tparams.file_properties.dest_path_buf.push(source_name);
        }
        let dest_path_str = self.tparams.file_properties.dest_path_buf.to_str().unwrap();
        if self.vfs.exists(dest_path_str) {
            self.vfs.truncate_file(dest_path_str)?;
        } else {
            self.vfs.create_file(dest_path_str)?;
        }
        self.tparams.tstate.file_status = FileStatus::Retained;
        self.step = TransactionStep::ReceivingFileDataPdus;
        Ok(())
    }

    fn transfer_completion(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<(), DestError> {
        self.notice_of_completion(cfdp_user)?;
        if self.tparams.transmission_mode() == TransmissionMode::Acknowledged
            || self.tparams.metadata_params().closure_requested
        {
            self.prepare_finished_pdu()?;
            self.step = TransactionStep::SendingFinishedPdu;
        } else {
            self.reset();
        }
        Ok(())
    }

    fn notice_of_completion(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<(), DestError> {
        if self.tstate().completion_disposition == CompletionDisposition::Completed {
            // TODO: Execute any filestore requests
        } else if self
            .tparams
            .remote_cfg
            .as_ref()
            .unwrap()
            .disposition_on_cancellation
            && self.tstate().delivery_code == DeliveryCode::Incomplete
        {
            self.vfs
                .remove_file(self.tparams.file_properties.dest_path_buf.to_str().unwrap())?;
            self.tstate_mut().file_status = FileStatus::DiscardDeliberately;
        }
        let tstate = self.tstate();
        let transaction_finished_params = TransactionFinishedParams {
            id: tstate.transaction_id.unwrap(),
            condition_code: tstate.condition_code,
            delivery_code: tstate.delivery_code,
            file_status: tstate.file_status,
        };
        cfdp_user.transaction_finished_indication(&transaction_finished_params);
        Ok(())
    }

    fn declare_fault(&mut self, condition_code: ConditionCode) -> FaultHandlerCode {
        // Cache those, because they might be reset when abandoning the transaction.
        let transaction_id = self.tstate().transaction_id.unwrap();
        let progress = self.tstate().progress;
        let fh_code = self
            .local_cfg
            .default_fault_handler
            .get_fault_handler(condition_code);
        match fh_code {
            FaultHandlerCode::NoticeOfCancellation => {
                self.notice_of_cancellation(condition_code);
            }
            FaultHandlerCode::NoticeOfSuspension => self.notice_of_suspension(),
            FaultHandlerCode::IgnoreError => (),
            FaultHandlerCode::AbandonTransaction => self.abandon_transaction(),
        }
        self.local_cfg
            .default_fault_handler
            .report_fault(transaction_id, condition_code, progress)
    }

    fn notice_of_cancellation(&mut self, condition_code: ConditionCode) {
        self.step = TransactionStep::TransferCompletion;
        self.tstate_mut().condition_code = condition_code;
        self.tstate_mut().completion_disposition = CompletionDisposition::Cancelled;
    }

    fn notice_of_suspension(&mut self) {
        // TODO: Implement suspension handling.
    }
    fn abandon_transaction(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        self.step = TransactionStep::Idle;
        self.state = State::Idle;
        self.packets_to_send_ctx.packet_available = false;
        self.tparams.reset();
    }

    fn prepare_finished_pdu(&mut self) -> Result<(), DestError> {
        self.packets_to_send_ctx.packet_available = true;
        self.packets_to_send_ctx.directive = Some(FileDirectiveType::FinishedPdu);
        self.step = TransactionStep::SendingFinishedPdu;
        Ok(())
    }

    fn tstate(&self) -> &TransferState {
        &self.tparams.tstate
    }

    fn tstate_mut(&mut self) -> &mut TransferState {
        &mut self.tparams.tstate
    }
}

#[cfg(test)]
mod tests {
    use core::{cell::Cell, sync::atomic::AtomicBool};
    #[allow(unused_imports)]
    use std::println;
    use std::{fs, sync::Mutex};

    use alloc::{collections::VecDeque, string::String, sync::Arc, vec::Vec};
    use rand::Rng;
    use spacepackets::{
        cfdp::{
            lv::Lv,
            pdu::{metadata::MetadataPduCreator, WritablePduPacket},
            ChecksumType, TransmissionMode,
        },
        util::{UbfU16, UnsignedByteFieldU16},
    };

    use crate::cfdp::{
        filestore::NativeFilestore, user::OwnedMetadataRecvdParams, CheckTimer, CheckTimerCreator,
        DefaultFaultHandler, IndicationConfig, RemoteEntityConfig, StdRemoteEntityConfigProvider,
        UserFaultHandler, CRC_32,
    };

    use super::*;

    const LOCAL_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(1);
    const REMOTE_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(2);

    pub struct FileSegmentRecvdParamsNoSegMetadata {
        pub id: TransactionId,
        pub offset: u64,
        pub length: usize,
    }

    #[derive(Default)]
    struct TestCfdpUser {
        next_expected_seq_num: u64,
        expected_full_src_name: String,
        expected_full_dest_name: String,
        expected_file_size: u64,
        transaction_indication_call_count: u32,
        eof_recvd_call_count: u32,
        finished_indic_queue: VecDeque<TransactionFinishedParams>,
        metadata_recv_queue: VecDeque<OwnedMetadataRecvdParams>,
        file_seg_recvd_queue: VecDeque<FileSegmentRecvdParamsNoSegMetadata>,
    }

    impl TestCfdpUser {
        fn new(
            next_expected_seq_num: u64,
            expected_full_src_name: String,
            expected_full_dest_name: String,
            expected_file_size: u64,
        ) -> Self {
            Self {
                next_expected_seq_num,
                expected_full_src_name,
                expected_full_dest_name,
                expected_file_size,
                transaction_indication_call_count: 0,
                eof_recvd_call_count: 0,
                finished_indic_queue: VecDeque::new(),
                metadata_recv_queue: VecDeque::new(),
                file_seg_recvd_queue: VecDeque::new(),
            }
        }

        fn generic_id_check(&self, id: &crate::cfdp::TransactionId) {
            assert_eq!(id.source_id, LOCAL_ID.into());
            assert_eq!(id.seq_num().value(), self.next_expected_seq_num);
        }
    }

    impl CfdpUser for TestCfdpUser {
        fn transaction_indication(&mut self, id: &crate::cfdp::TransactionId) {
            self.generic_id_check(id);
            self.transaction_indication_call_count += 1;
        }

        fn eof_sent_indication(&mut self, id: &crate::cfdp::TransactionId) {
            self.generic_id_check(id);
        }

        fn transaction_finished_indication(
            &mut self,
            finished_params: &crate::cfdp::user::TransactionFinishedParams,
        ) {
            self.generic_id_check(&finished_params.id);
            self.finished_indic_queue.push_back(*finished_params);
        }

        fn metadata_recvd_indication(
            &mut self,
            md_recvd_params: &crate::cfdp::user::MetadataReceivedParams,
        ) {
            self.generic_id_check(&md_recvd_params.id);
            assert_eq!(
                String::from(md_recvd_params.src_file_name),
                self.expected_full_src_name
            );
            assert_eq!(
                String::from(md_recvd_params.dest_file_name),
                self.expected_full_dest_name
            );
            assert_eq!(md_recvd_params.msgs_to_user.len(), 0);
            assert_eq!(md_recvd_params.source_id, LOCAL_ID.into());
            assert_eq!(md_recvd_params.file_size, self.expected_file_size);
            self.metadata_recv_queue.push_back(md_recvd_params.into());
        }

        fn file_segment_recvd_indication(
            &mut self,
            segment_recvd_params: &crate::cfdp::user::FileSegmentRecvdParams,
        ) {
            self.generic_id_check(&segment_recvd_params.id);
            self.file_seg_recvd_queue
                .push_back(FileSegmentRecvdParamsNoSegMetadata {
                    id: segment_recvd_params.id,
                    offset: segment_recvd_params.offset,
                    length: segment_recvd_params.length,
                })
        }

        fn report_indication(&mut self, _id: &crate::cfdp::TransactionId) {}

        fn suspended_indication(
            &mut self,
            _id: &crate::cfdp::TransactionId,
            _condition_code: ConditionCode,
        ) {
            panic!("unexpected suspended indication");
        }

        fn resumed_indication(&mut self, _id: &crate::cfdp::TransactionId, _progresss: u64) {}

        fn fault_indication(
            &mut self,
            _id: &crate::cfdp::TransactionId,
            _condition_code: ConditionCode,
            _progress: u64,
        ) {
            panic!("unexpected fault indication");
        }

        fn abandoned_indication(
            &mut self,
            _id: &crate::cfdp::TransactionId,
            _condition_code: ConditionCode,
            _progress: u64,
        ) {
            panic!("unexpected abandoned indication");
        }

        fn eof_recvd_indication(&mut self, id: &crate::cfdp::TransactionId) {
            self.generic_id_check(id);
            self.eof_recvd_call_count += 1;
        }
    }

    #[derive(Default, Clone)]
    struct TestFaultHandler {
        notice_of_suspension_queue: Arc<Mutex<VecDeque<(TransactionId, ConditionCode, u64)>>>,
        notice_of_cancellation_queue: Arc<Mutex<VecDeque<(TransactionId, ConditionCode, u64)>>>,
        abandoned_queue: Arc<Mutex<VecDeque<(TransactionId, ConditionCode, u64)>>>,
        ignored_queue: Arc<Mutex<VecDeque<(TransactionId, ConditionCode, u64)>>>,
    }

    impl UserFaultHandler for TestFaultHandler {
        fn notice_of_suspension_cb(
            &mut self,
            transaction_id: TransactionId,
            cond: ConditionCode,
            progress: u64,
        ) {
            self.notice_of_suspension_queue.lock().unwrap().push_back((
                transaction_id,
                cond,
                progress,
            ))
        }

        fn notice_of_cancellation_cb(
            &mut self,
            transaction_id: TransactionId,
            cond: ConditionCode,
            progress: u64,
        ) {
            self.notice_of_cancellation_queue
                .lock()
                .unwrap()
                .push_back((transaction_id, cond, progress))
        }

        fn abandoned_cb(
            &mut self,
            transaction_id: TransactionId,
            cond: ConditionCode,
            progress: u64,
        ) {
            self.abandoned_queue
                .lock()
                .unwrap()
                .push_back((transaction_id, cond, progress))
        }

        fn ignore_cb(&mut self, transaction_id: TransactionId, cond: ConditionCode, progress: u64) {
            self.ignored_queue
                .lock()
                .unwrap()
                .push_back((transaction_id, cond, progress))
        }
    }

    impl TestFaultHandler {
        fn suspension_queue_empty(&self) -> bool {
            self.notice_of_suspension_queue.lock().unwrap().is_empty()
        }
        fn cancellation_queue_empty(&self) -> bool {
            self.notice_of_cancellation_queue.lock().unwrap().is_empty()
        }
        fn ignored_queue_empty(&self) -> bool {
            self.ignored_queue.lock().unwrap().is_empty()
        }
        fn abandoned_queue_empty(&self) -> bool {
            self.abandoned_queue.lock().unwrap().is_empty()
        }
        fn all_queues_empty(&self) -> bool {
            self.suspension_queue_empty()
                && self.cancellation_queue_empty()
                && self.ignored_queue_empty()
                && self.abandoned_queue_empty()
        }
    }

    #[derive(Debug)]
    struct TestCheckTimer {
        counter: Cell<u32>,
        expired: Arc<AtomicBool>,
    }

    impl CheckTimer for TestCheckTimer {
        fn has_expired(&self) -> bool {
            self.expired.load(core::sync::atomic::Ordering::Relaxed)
        }
        fn reset(&mut self) {
            self.counter.set(0);
        }
    }

    impl TestCheckTimer {
        pub fn new(expired_flag: Arc<AtomicBool>) -> Self {
            Self {
                counter: Cell::new(0),
                expired: expired_flag,
            }
        }
    }

    struct TestCheckTimerCreator {
        expired_flag: Arc<AtomicBool>,
    }

    impl TestCheckTimerCreator {
        pub fn new(expired_flag: Arc<AtomicBool>) -> Self {
            Self { expired_flag }
        }
    }

    impl CheckTimerCreator for TestCheckTimerCreator {
        fn get_check_timer_provider(
            &self,
            _local_id: &UnsignedByteField,
            _remote_id: &UnsignedByteField,
            _entity_type: crate::cfdp::EntityType,
        ) -> Box<dyn CheckTimer> {
            Box::new(TestCheckTimer::new(self.expired_flag.clone()))
        }
    }

    struct DestHandlerTester {
        check_timer_expired: Arc<AtomicBool>,
        handler: DestinationHandler,
        src_path: PathBuf,
        dest_path: PathBuf,
        check_dest_file: bool,
        check_handler_idle_at_drop: bool,
        expected_file_size: u64,
        pdu_header: PduHeader,
        expected_full_data: Vec<u8>,
        buf: [u8; 512],
    }

    impl DestHandlerTester {
        fn new(fault_handler: TestFaultHandler) -> Self {
            let check_timer_expired = Arc::new(AtomicBool::new(false));
            let dest_handler = default_dest_handler(fault_handler, check_timer_expired.clone());
            let (src_path, dest_path) = init_full_filenames();
            assert!(!Path::exists(&dest_path));
            let handler = Self {
                check_timer_expired,
                handler: dest_handler,
                src_path,
                dest_path,
                check_dest_file: false,
                check_handler_idle_at_drop: false,
                expected_file_size: 0,
                pdu_header: create_pdu_header(UbfU16::new(0)),
                expected_full_data: Vec::new(),
                buf: [0; 512],
            };
            handler.state_check(State::Idle, TransactionStep::Idle);
            handler
        }

        fn dest_path(&self) -> &PathBuf {
            &self.dest_path
        }

        #[allow(dead_code)]
        fn indication_cfg_mut(&mut self) -> &mut IndicationConfig {
            &mut self.handler.local_cfg.indication_cfg
        }

        fn indication_cfg(&mut self) -> &IndicationConfig {
            &self.handler.local_cfg.indication_cfg
        }

        fn set_check_timer_expired(&mut self) {
            self.check_timer_expired
                .store(true, core::sync::atomic::Ordering::Relaxed);
        }

        fn test_user_from_cached_paths(&self, expected_file_size: u64) -> TestCfdpUser {
            TestCfdpUser::new(
                0,
                self.src_path.to_string_lossy().into(),
                self.dest_path.to_string_lossy().into(),
                expected_file_size,
            )
        }

        fn generic_transfer_init(
            &mut self,
            user: &mut TestCfdpUser,
            file_size: u64,
        ) -> Result<TransactionId, DestError> {
            self.expected_file_size = file_size;
            let metadata_pdu = create_metadata_pdu(
                &self.pdu_header,
                self.src_path.as_path(),
                self.dest_path.as_path(),
                file_size,
            );
            let packet_info = create_packet_info(&metadata_pdu, &mut self.buf);
            self.handler.state_machine(user, Some(&packet_info))?;
            assert_eq!(user.metadata_recv_queue.len(), 1);
            assert_eq!(
                self.handler.transmission_mode().unwrap(),
                TransmissionMode::Unacknowledged
            );
            Ok(self.handler.transaction_id().unwrap())
        }

        fn generic_file_data_insert(
            &mut self,
            user: &mut TestCfdpUser,
            offset: u64,
            file_data_chunk: &[u8],
        ) -> Result<(), DestError> {
            let filedata_pdu =
                FileDataPdu::new_no_seg_metadata(self.pdu_header, offset, file_data_chunk);
            filedata_pdu
                .write_to_bytes(&mut self.buf)
                .expect("writing file data PDU failed");
            let packet_info = PacketInfo::new(&self.buf).expect("creating packet info failed");
            let result = self.handler.state_machine(user, Some(&packet_info));
            if self.indication_cfg().file_segment_recv {
                assert!(!user.file_seg_recvd_queue.is_empty());
                assert_eq!(user.file_seg_recvd_queue.back().unwrap().offset, offset);
                assert_eq!(
                    user.file_seg_recvd_queue.back().unwrap().length,
                    file_data_chunk.len()
                );
            }
            result
        }

        fn generic_eof_no_error(
            &mut self,
            user: &mut TestCfdpUser,
            expected_full_data: Vec<u8>,
        ) -> Result<(), DestError> {
            self.expected_full_data = expected_full_data;
            let eof_pdu = create_no_error_eof(&self.expected_full_data, &self.pdu_header);
            let packet_info = create_packet_info(&eof_pdu, &mut self.buf);
            self.check_handler_idle_at_drop = true;
            self.check_dest_file = true;
            let result = self.handler.state_machine(user, Some(&packet_info));
            if self.indication_cfg().eof_recv {
                assert_eq!(user.eof_recvd_call_count, 1);
            }
            result
        }

        fn state_check(&self, state: State, step: TransactionStep) {
            assert_eq!(self.handler.state(), state);
            assert_eq!(self.handler.step(), step);
        }
    }

    impl Drop for DestHandlerTester {
        fn drop(&mut self) {
            if self.check_handler_idle_at_drop {
                self.state_check(State::Idle, TransactionStep::Idle);
            }
            if self.check_dest_file {
                assert!(Path::exists(&self.dest_path));
                let read_content = fs::read(&self.dest_path).expect("reading back string failed");
                assert_eq!(read_content.len() as u64, self.expected_file_size);
                assert_eq!(read_content, self.expected_full_data);
                assert!(fs::remove_file(self.dest_path.as_path()).is_ok());
            }
        }
    }

    fn init_full_filenames() -> (PathBuf, PathBuf) {
        (
            tempfile::TempPath::from_path("/tmp/test.txt").to_path_buf(),
            tempfile::NamedTempFile::new()
                .unwrap()
                .into_temp_path()
                .to_path_buf(),
        )
    }

    fn basic_remote_cfg_table() -> StdRemoteEntityConfigProvider {
        let mut table = StdRemoteEntityConfigProvider::default();
        let remote_entity_cfg = RemoteEntityConfig::new_with_default_values(
            UnsignedByteFieldU16::new(1).into(),
            1024,
            1024,
            true,
            true,
            TransmissionMode::Unacknowledged,
            ChecksumType::Crc32,
        );
        table.add_config(&remote_entity_cfg);
        table
    }

    fn default_dest_handler(
        test_fault_handler: TestFaultHandler,
        check_timer_expired: Arc<AtomicBool>,
    ) -> DestinationHandler {
        let local_entity_cfg = LocalEntityConfig {
            id: REMOTE_ID.into(),
            indication_cfg: IndicationConfig::default(),
            default_fault_handler: DefaultFaultHandler::new(Box::new(test_fault_handler)),
        };
        DestinationHandler::new(
            local_entity_cfg,
            Box::<NativeFilestore>::default(),
            Box::new(basic_remote_cfg_table()),
            Box::new(TestCheckTimerCreator::new(check_timer_expired)),
        )
    }

    fn create_pdu_header(seq_num: impl Into<UnsignedByteField>) -> PduHeader {
        let mut pdu_conf =
            CommonPduConfig::new_with_byte_fields(LOCAL_ID, REMOTE_ID, seq_num).unwrap();
        pdu_conf.trans_mode = TransmissionMode::Unacknowledged;
        PduHeader::new_no_file_data(pdu_conf, 0)
    }

    fn create_metadata_pdu<'filename>(
        pdu_header: &PduHeader,
        src_name: &'filename Path,
        dest_name: &'filename Path,
        file_size: u64,
    ) -> MetadataPduCreator<'filename, 'filename, 'static> {
        let checksum_type = if file_size == 0 {
            ChecksumType::NullChecksum
        } else {
            ChecksumType::Crc32
        };
        let metadata_params = MetadataGenericParams::new(false, checksum_type, file_size);
        MetadataPduCreator::new_no_opts(
            *pdu_header,
            metadata_params,
            Lv::new_from_str(src_name.as_os_str().to_str().unwrap()).unwrap(),
            Lv::new_from_str(dest_name.as_os_str().to_str().unwrap()).unwrap(),
        )
    }

    fn create_packet_info<'a>(
        pdu: &'a impl WritablePduPacket,
        buf: &'a mut [u8],
    ) -> PacketInfo<'a> {
        let written_len = pdu
            .write_to_bytes(buf)
            .expect("writing metadata PDU failed");
        PacketInfo::new(&buf[..written_len]).expect("generating packet info failed")
    }

    fn create_no_error_eof(file_data: &[u8], pdu_header: &PduHeader) -> EofPdu {
        let crc32 = if !file_data.is_empty() {
            let mut digest = CRC_32.digest();
            digest.update(file_data);
            digest.finalize()
        } else {
            0
        };
        EofPdu::new_no_error(*pdu_header, crc32, file_data.len() as u64)
    }

    #[test]
    fn test_basic() {
        let fault_handler = TestFaultHandler::default();
        let dest_handler = default_dest_handler(fault_handler.clone(), Arc::default());
        assert!(dest_handler.transmission_mode().is_none());
        assert!(fault_handler.all_queues_empty());
    }

    #[test]
    fn test_empty_file_transfer_not_acked() {
        let fault_handler = TestFaultHandler::default();
        let mut test_obj = DestHandlerTester::new(fault_handler.clone());
        let mut test_user = test_obj.test_user_from_cached_paths(0);
        test_obj
            .generic_transfer_init(&mut test_user, 0)
            .expect("transfer init failed");
        test_obj.state_check(State::Busy, TransactionStep::ReceivingFileDataPdus);
        test_obj
            .generic_eof_no_error(&mut test_user, Vec::new())
            .expect("EOF no error insertion failed");
        assert!(fault_handler.all_queues_empty());
    }

    #[test]
    fn test_small_file_transfer_not_acked() {
        let file_data_str = "Hello World!";
        let file_data = file_data_str.as_bytes();
        let file_size = file_data.len() as u64;
        let fault_handler = TestFaultHandler::default();

        let mut test_obj = DestHandlerTester::new(fault_handler.clone());
        let mut test_user = test_obj.test_user_from_cached_paths(file_size);
        test_obj
            .generic_transfer_init(&mut test_user, file_size)
            .expect("transfer init failed");
        test_obj.state_check(State::Busy, TransactionStep::ReceivingFileDataPdus);
        test_obj
            .generic_file_data_insert(&mut test_user, 0, file_data)
            .expect("file data insertion failed");
        test_obj
            .generic_eof_no_error(&mut test_user, file_data.to_vec())
            .expect("EOF no error insertion failed");
        assert!(fault_handler.all_queues_empty());
    }

    #[test]
    fn test_segmented_file_transfer_not_acked() {
        let mut rng = rand::thread_rng();
        let mut random_data = [0u8; 512];
        rng.fill(&mut random_data);
        let file_size = random_data.len() as u64;
        let segment_len = 256;
        let fault_handler = TestFaultHandler::default();

        let mut test_obj = DestHandlerTester::new(fault_handler.clone());
        let mut test_user = test_obj.test_user_from_cached_paths(file_size);
        test_obj
            .generic_transfer_init(&mut test_user, file_size)
            .expect("transfer init failed");
        test_obj.state_check(State::Busy, TransactionStep::ReceivingFileDataPdus);
        test_obj
            .generic_file_data_insert(&mut test_user, 0, &random_data[0..segment_len])
            .expect("file data insertion failed");
        test_obj
            .generic_file_data_insert(
                &mut test_user,
                segment_len as u64,
                &random_data[segment_len..],
            )
            .expect("file data insertion failed");
        test_obj
            .generic_eof_no_error(&mut test_user, random_data.to_vec())
            .expect("EOF no error insertion failed");
        assert!(fault_handler.all_queues_empty());
    }

    #[test]
    fn test_check_limit_handling_transfer_success() {
        let mut rng = rand::thread_rng();
        let mut random_data = [0u8; 512];
        rng.fill(&mut random_data);
        let file_size = random_data.len() as u64;
        let segment_len = 256;
        let fault_handler = TestFaultHandler::default();

        let mut test_obj = DestHandlerTester::new(fault_handler.clone());
        let mut test_user = test_obj.test_user_from_cached_paths(file_size);
        let transaction_id = test_obj
            .generic_transfer_init(&mut test_user, file_size)
            .expect("transfer init failed");

        test_obj.state_check(State::Busy, TransactionStep::ReceivingFileDataPdus);
        test_obj
            .generic_file_data_insert(&mut test_user, 0, &random_data[0..segment_len])
            .expect("file data insertion 0 failed");
        test_obj
            .generic_eof_no_error(&mut test_user, random_data.to_vec())
            .expect("EOF no error insertion failed");
        test_obj.state_check(
            State::Busy,
            TransactionStep::ReceivingFileDataPdusWithCheckLimitHandling,
        );
        test_obj
            .generic_file_data_insert(
                &mut test_user,
                segment_len as u64,
                &random_data[segment_len..],
            )
            .expect("file data insertion 1 failed");
        test_obj.set_check_timer_expired();
        test_obj
            .handler
            .state_machine(&mut test_user, None)
            .expect("fsm failure");

        let ignored_queue = fault_handler.ignored_queue.lock().unwrap();
        assert_eq!(ignored_queue.len(), 1);
        let cancelled = *ignored_queue.front().unwrap();
        assert_eq!(cancelled.0, transaction_id);
        assert_eq!(cancelled.1, ConditionCode::FileChecksumFailure);
        assert_eq!(cancelled.2, segment_len as u64);
    }

    #[test]
    fn test_check_limit_handling_limit_reached() {
        let mut rng = rand::thread_rng();
        let mut random_data = [0u8; 512];
        rng.fill(&mut random_data);
        let file_size = random_data.len() as u64;
        let segment_len = 256;

        let fault_handler = TestFaultHandler::default();
        let mut test_obj = DestHandlerTester::new(fault_handler.clone());
        let mut test_user = test_obj.test_user_from_cached_paths(file_size);
        let transaction_id = test_obj
            .generic_transfer_init(&mut test_user, file_size)
            .expect("transfer init failed");

        test_obj.state_check(State::Busy, TransactionStep::ReceivingFileDataPdus);
        test_obj
            .generic_file_data_insert(&mut test_user, 0, &random_data[0..segment_len])
            .expect("file data insertion 0 failed");
        test_obj
            .generic_eof_no_error(&mut test_user, random_data.to_vec())
            .expect("EOF no error insertion failed");
        test_obj.state_check(
            State::Busy,
            TransactionStep::ReceivingFileDataPdusWithCheckLimitHandling,
        );
        test_obj.set_check_timer_expired();
        test_obj
            .handler
            .state_machine(&mut test_user, None)
            .expect("fsm error");
        test_obj.state_check(
            State::Busy,
            TransactionStep::ReceivingFileDataPdusWithCheckLimitHandling,
        );
        test_obj.set_check_timer_expired();
        test_obj
            .handler
            .state_machine(&mut test_user, None)
            .expect("fsm error");
        test_obj.state_check(State::Idle, TransactionStep::Idle);

        assert!(fault_handler
            .notice_of_suspension_queue
            .lock()
            .unwrap()
            .is_empty());

        let ignored_queue = fault_handler.ignored_queue.lock().unwrap();
        assert_eq!(ignored_queue.len(), 1);
        let cancelled = *ignored_queue.front().unwrap();
        assert_eq!(cancelled.0, transaction_id);
        assert_eq!(cancelled.1, ConditionCode::FileChecksumFailure);
        assert_eq!(cancelled.2, segment_len as u64);

        let cancelled_queue = fault_handler.notice_of_cancellation_queue.lock().unwrap();
        assert_eq!(cancelled_queue.len(), 1);
        let cancelled = *cancelled_queue.front().unwrap();
        assert_eq!(cancelled.0, transaction_id);
        assert_eq!(cancelled.1, ConditionCode::CheckLimitReached);
        assert_eq!(cancelled.2, segment_len as u64);

        drop(cancelled_queue);

        // Check that the broken file exists.
        test_obj.check_dest_file = false;
        assert!(Path::exists(test_obj.dest_path()));
        let read_content = fs::read(test_obj.dest_path()).expect("reading back string failed");
        assert_eq!(read_content.len(), segment_len);
        assert_eq!(read_content, &random_data[0..segment_len]);
        assert!(fs::remove_file(test_obj.dest_path().as_path()).is_ok());
    }
}
