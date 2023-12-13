use core::str::{from_utf8, Utf8Error};
use std::{
    fs::{metadata, File},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::cfdp::user::TransactionFinishedParams;

use super::{
    filestore::{FilestoreError, VirtualFilestore},
    user::{CfdpUser, MetadataReceivedParams},
    CheckTimerCreator, LocalEntityConfig, PacketInfo, PacketTarget, RemoteEntityConfig,
    RemoteEntityConfigProvider, State, TransactionId, TransactionStep,
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

#[derive(Debug)]
struct TransferState {
    transaction_id: Option<TransactionId>,
    progress: u64,
    condition_code: ConditionCode,
    delivery_code: DeliveryCode,
    file_status: FileStatus,
    metadata_params: MetadataGenericParams,
}

impl Default for TransferState {
    fn default() -> Self {
        Self {
            transaction_id: None,
            progress: Default::default(),
            condition_code: ConditionCode::NoError,
            delivery_code: DeliveryCode::Incomplete,
            file_status: FileStatus::Unreported,
            metadata_params: Default::default(),
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
    #[error("pdu error {0}")]
    Pdu(#[from] PduError),
    #[error("io error {0}")]
    Io(#[from] std::io::Error),
    #[error("path conversion error {0}")]
    PathConversion(#[from] Utf8Error),
    #[error("error building dest path from source file name and dest folder")]
    PathConcatError,
    #[error("no remote entity configuration found for {0:?}")]
    NoRemoteCfgFound(UnsignedByteField),
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

    pub fn state_machine(
        &mut self,
        cfdp_user: &mut impl CfdpUser,
        packet_to_insert: Option<&PacketInfo>,
    ) -> Result<(), DestError> {
        if let Some(packet) = packet_to_insert {
            self.insert_packet(cfdp_user, packet)?;
        }
        match self.state {
            State::Idle => todo!(),
            State::Busy => self.fsm_busy(cfdp_user),
            State::Suspended => todo!(),
        }
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
            PduType::FileData => self.handle_file_data(packet_info.raw_packet),
        }
    }

    pub fn packet_to_send_ready(&self) -> bool {
        self.packets_to_send_ctx.packet_available
    }

    pub fn get_next_packet_to_send(
        &self,
        buf: &mut [u8],
    ) -> Result<Option<(FileDirectiveType, usize)>, DestError> {
        if !self.packet_to_send_ready() {
            return Ok(None);
        }
        let directive = self.packets_to_send_ctx.directive.unwrap();
        let written_size = match directive {
            FileDirectiveType::FinishedPdu => {
                let pdu_header = PduHeader::new_no_file_data(self.tparams.pdu_conf, 0);
                let finished_pdu = if self.tparams.tstate.condition_code == ConditionCode::NoError
                    || self.tparams.tstate.condition_code == ConditionCode::UnsupportedChecksumType
                {
                    FinishedPduCreator::new_default(
                        pdu_header,
                        self.tparams.tstate.delivery_code,
                        self.tparams.tstate.file_status,
                    )
                } else {
                    // TODO: Are there cases where this ID is actually the source entity ID?
                    let entity_id = EntityIdTlv::new(self.local_cfg.id);
                    FinishedPduCreator::new_with_error(
                        pdu_header,
                        self.tparams.tstate.condition_code,
                        self.tparams.tstate.delivery_code,
                        self.tparams.tstate.file_status,
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

    pub fn handle_file_directive(
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

    pub fn handle_metadata_pdu(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
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

    pub fn handle_file_data(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
        if self.state == State::Idle || self.step != TransactionStep::ReceivingFileDataPdus {
            return Err(DestError::WrongStateForFileDataAndEof);
        }
        let fd_pdu = FileDataPdu::from_bytes(raw_packet)?;
        let mut dest_file = File::options()
            .write(true)
            .open(&self.tparams.file_properties.dest_path_buf)?;
        dest_file.seek(SeekFrom::Start(fd_pdu.offset()))?;
        dest_file.write_all(fd_pdu.file_data())?;
        Ok(())
    }

    pub fn handle_eof_pdu(
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
        if eof_pdu.condition_code() == ConditionCode::NoError {
            self.handle_no_error_eof_pdu(&eof_pdu)?;
        } else {
            todo!("implement cancel request handling");
        }
        Ok(())
    }

    fn handle_no_error_eof_pdu(&mut self, eof_pdu: &EofPdu) -> Result<bool, DestError> {
        // CFDP 4.6.1.2.9: Declare file size error if progress exceeds file size
        if self.tparams.tstate.progress > eof_pdu.file_size()
            && self.declare_fault(ConditionCode::FileSizeError) != FaultHandlerCode::IgnoreError
        {
            return Ok(true);
        }

        if self.tparams.transmission_mode() == TransmissionMode::Unacknowledged
            && !self.checksum_verify(eof_pdu.file_checksum())
        {
            self.start_check_limit_handling()
        }

        // TODO: Continue here based on Python implementation.

        // For a standard disk based file system, which is assumed to be used for now, the file
        // will always be retained. This might change in the future.
        /*
        self.tparams.tstate.file_status = FileStatus::Retained;
        if self.checksum_verify(eof_pdu.file_checksum())? {
            self.tparams.tstate.condition_code = ConditionCode::NoError;
            self.tparams.tstate.delivery_code = DeliveryCode::Complete;
        } else {
            self.tparams.tstate.condition_code = ConditionCode::FileChecksumFailure;
        }
        */
        // TODO: Check progress, and implement transfer completion timer as specified in the
        // standard. This timer protects against out of order arrival of packets.
        // if self.tparams.tstate.progress != self.tparams.file_size() {}
        if self.tparams.transmission_mode() == TransmissionMode::Unacknowledged {
            self.step = TransactionStep::TransferCompletion;
        } else {
            self.step = TransactionStep::SendingAckPdu;
        }
        Ok(false)
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
                FilestoreError::ChecksumTypeNotImplemented(checksum) => {
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

    fn start_check_limit_handling(&mut self) {}
    pub fn handle_prompt_pdu(&mut self, _raw_packet: &[u8]) -> Result<(), DestError> {
        todo!();
    }

    fn fsm_busy(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<(), DestError> {
        if self.step == TransactionStep::TransactionStart {
            self.transaction_start(cfdp_user)?;
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

        if dest_path.exists() {
            let dest_metadata = metadata(dest_path)?;
            if dest_metadata.is_dir() {
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
                    return Err(DestError::PathConcatError);
                }
                let source_name = source_name.unwrap();
                self.tparams.file_properties.dest_path_buf.push(source_name);
            }
        }
        // This function does exactly what we require: Create a new file if it does not exist yet
        // and trucate an existing one.
        File::create(&self.tparams.file_properties.dest_path_buf)?;
        self.step = TransactionStep::ReceivingFileDataPdus;
        Ok(())
    }

    fn transfer_completion(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<(), DestError> {
        let transaction_finished_params = TransactionFinishedParams {
            id: self.tparams.tstate.transaction_id.unwrap(),
            condition_code: self.tparams.tstate.condition_code,
            delivery_code: self.tparams.tstate.delivery_code,
            file_status: self.tparams.tstate.file_status,
        };
        cfdp_user.transaction_finished_indication(&transaction_finished_params);
        // This function should never be called with metadata parameters not set
        if self.tparams.metadata_params().closure_requested {
            self.prepare_finished_pdu()?;
            self.step = TransactionStep::SendingFinishedPdu;
        } else {
            self.reset();
            self.state = State::Idle;
            self.step = TransactionStep::Idle;
        }
        Ok(())
    }

    fn declare_fault(&mut self, condition_code: ConditionCode) -> FaultHandlerCode {
        let fh_code = self
            .local_cfg
            .default_fault_handler
            .get_fault_handler(condition_code);
        match fh_code {
            FaultHandlerCode::NoticeOfCancellation => {
                self.notice_of_cancellation();
            }
            FaultHandlerCode::NoticeOfSuspension => self.notice_of_suspension(),
            FaultHandlerCode::IgnoreError => todo!(),
            FaultHandlerCode::AbandonTransaction => todo!(),
        }
        self.local_cfg.default_fault_handler.report_fault(
            self.tparams.tstate.transaction_id.unwrap(),
            condition_code,
            self.tparams.tstate.progress,
        )
    }

    fn notice_of_cancellation(&mut self) {}
    fn notice_of_suspension(&mut self) {}

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
}

#[cfg(test)]
mod tests {
    use core::{
        cell::Cell,
        sync::atomic::{AtomicU8, Ordering},
    };
    #[allow(unused_imports)]
    use std::println;
    use std::{env::temp_dir, fs};

    use alloc::{format, string::String};
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
        filestore::NativeFilestore, CheckTimer, CheckTimerCreator, DefaultFaultHandler,
        IndicationConfig, RemoteEntityConfig, StdRemoteEntityConfigProvider, UserFaultHandler,
        CRC_32,
    };

    use super::*;

    const LOCAL_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(1);
    const REMOTE_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(2);

    #[derive(Default)]
    struct TestCfdpUser {
        next_expected_seq_num: u64,
        expected_full_src_name: String,
        expected_full_dest_name: String,
        expected_file_size: usize,
    }

    impl TestCfdpUser {
        fn generic_id_check(&self, id: &crate::cfdp::TransactionId) {
            assert_eq!(id.source_id, LOCAL_ID.into());
            assert_eq!(id.seq_num().value(), self.next_expected_seq_num);
        }
    }

    #[derive(Default)]
    struct TestFaultHandler {
        notice_of_suspension_count: u32,
        notice_of_cancellation_count: u32,
        abandoned_count: u32,
        ignored_count: u32,
    }

    impl UserFaultHandler for TestFaultHandler {
        fn notice_of_suspension_cb(
            &mut self,
            transaction_id: TransactionId,
            cond: ConditionCode,
            progress: u64,
        ) {
            self.notice_of_suspension_count += 1;
        }

        fn notice_of_cancellation_cb(
            &mut self,
            transaction_id: TransactionId,
            cond: ConditionCode,
            progress: u64,
        ) {
            self.notice_of_cancellation_count += 1;
        }

        fn abandoned_cb(
            &mut self,
            transaction_id: TransactionId,
            cond: ConditionCode,
            progress: u64,
        ) {
            self.abandoned_count += 1;
        }

        fn ignore_cb(&mut self, transaction_id: TransactionId, cond: ConditionCode, progress: u64) {
            self.ignored_count += 1;
        }
    }

    impl CfdpUser for TestCfdpUser {
        fn transaction_indication(&mut self, id: &crate::cfdp::TransactionId) {
            self.generic_id_check(id);
        }

        fn eof_sent_indication(&mut self, id: &crate::cfdp::TransactionId) {
            self.generic_id_check(id);
        }

        fn transaction_finished_indication(
            &mut self,
            finished_params: &crate::cfdp::user::TransactionFinishedParams,
        ) {
            self.generic_id_check(&finished_params.id);
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
            assert_eq!(md_recvd_params.file_size as usize, self.expected_file_size);
        }

        fn file_segment_recvd_indication(
            &mut self,
            _segment_recvd_params: &crate::cfdp::user::FileSegmentRecvdParams,
        ) {
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
        }
    }

    struct TestCheckTimer {
        counter: Cell<u32>,
        expiry_count: u32,
    }

    impl CheckTimer for TestCheckTimer {
        fn has_expired(&self) -> bool {
            let current_counter = self.counter.get();
            if self.expiry_count == current_counter {
                return true;
            }
            self.counter.set(current_counter + 1);
            false
        }
    }

    impl TestCheckTimer {
        pub fn new(expiry_after_x_calls: u32) -> Self {
            Self {
                counter: Cell::new(0),
                expiry_count: expiry_after_x_calls,
            }
        }
    }

    struct TestCheckTimerCreator {
        expiry_counter_for_source_entity: u32,
        expiry_counter_for_dest_entity: u32,
    }

    impl TestCheckTimerCreator {
        pub fn new(
            expiry_counter_for_source_entity: u32,
            expiry_counter_for_dest_entity: u32,
        ) -> Self {
            Self {
                expiry_counter_for_source_entity,
                expiry_counter_for_dest_entity,
            }
        }
    }

    impl CheckTimerCreator for TestCheckTimerCreator {
        fn get_check_timer_provider(
            &self,
            _local_id: &UnsignedByteField,
            _remote_id: &UnsignedByteField,
            entity_type: crate::cfdp::EntityType,
        ) -> Box<dyn CheckTimer> {
            if entity_type == crate::cfdp::EntityType::Sending {
                Box::new(TestCheckTimer::new(self.expiry_counter_for_source_entity))
            } else {
                Box::new(TestCheckTimer::new(self.expiry_counter_for_dest_entity))
            }
        }
    }

    fn init_check(handler: &DestinationHandler) {
        assert_eq!(handler.state(), State::Idle);
        assert_eq!(handler.step(), TransactionStep::Idle);
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

    fn default_dest_handler() -> DestinationHandler {
        let test_fault_handler = TestFaultHandler::default();
        let local_entity_cfg = LocalEntityConfig {
            id: REMOTE_ID.into(),
            indication_cfg: IndicationConfig::default(),
            default_fault_handler: DefaultFaultHandler::new(Box::new(test_fault_handler)),
        };
        DestinationHandler::new(
            local_entity_cfg,
            Box::<NativeFilestore>::default(),
            Box::new(basic_remote_cfg_table()),
            Box::new(TestCheckTimerCreator::new(2, 2)),
        )
    }
    #[test]
    fn test_basic() {
        let dest_handler = default_dest_handler();
        init_check(&dest_handler);
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
        let metadata_params = MetadataGenericParams::new(false, ChecksumType::Crc32, file_size);
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
    fn create_no_error_eof(file_data: &[u8], pdu_header: &PduHeader, buf: &mut [u8]) -> EofPdu {
        let mut digest = CRC_32.digest();
        digest.update(file_data);
        let crc32 = digest.finalize();
        EofPdu::new_no_error(*pdu_header, crc32, file_data.len() as u64)
    }

    #[test]
    fn test_empty_file_transfer_not_acked() {
        let (src_name, dest_name) = init_full_filenames();
        assert!(!Path::exists(&dest_name));
        let mut buf: [u8; 512] = [0; 512];
        let mut test_user = TestCfdpUser {
            next_expected_seq_num: 0,
            expected_full_src_name: src_name.to_string_lossy().into(),
            expected_full_dest_name: dest_name.to_string_lossy().into(),
            expected_file_size: 0,
        };
        // We treat the destination handler like it is a remote entity.
        let mut dest_handler = default_dest_handler();
        init_check(&dest_handler);

        let seq_num = UbfU16::new(0);
        let pdu_header = create_pdu_header(seq_num);
        let metadata_pdu =
            create_metadata_pdu(&pdu_header, src_name.as_path(), dest_name.as_path(), 0);
        let packet_info = create_packet_info(&metadata_pdu, &mut buf);
        let result = dest_handler.state_machine(&mut test_user, Some(&packet_info));
        if let Err(e) = result {
            panic!("dest handler fsm error: {e}");
        }
        assert_ne!(dest_handler.state(), State::Idle);
        assert_eq!(dest_handler.step(), TransactionStep::ReceivingFileDataPdus);

        let eof_pdu = create_no_error_eof(&[], &pdu_header, &mut buf);
        let packet_info = create_packet_info(&eof_pdu, &mut buf);
        let result = dest_handler.state_machine(&mut test_user, Some(&packet_info));
        assert!(result.is_ok());
        assert_eq!(dest_handler.state(), State::Idle);
        assert_eq!(dest_handler.step(), TransactionStep::Idle);
        assert!(Path::exists(&dest_name));
        let read_content = fs::read(&dest_name).expect("reading back string failed");
        assert_eq!(read_content.len(), 0);
        assert!(fs::remove_file(dest_name).is_ok());
    }

    #[test]
    fn test_small_file_transfer_not_acked() {
        let (src_name, dest_name) = init_full_filenames();
        assert!(!Path::exists(&dest_name));
        let file_data_str = "Hello World!";
        let file_data = file_data_str.as_bytes();
        let mut buf: [u8; 512] = [0; 512];
        let mut test_user = TestCfdpUser {
            next_expected_seq_num: 0,
            expected_full_src_name: src_name.to_string_lossy().into(),
            expected_full_dest_name: dest_name.to_string_lossy().into(),
            expected_file_size: file_data.len(),
        };
        // We treat the destination handler like it is a remote entity.
        let mut dest_handler = default_dest_handler();
        init_check(&dest_handler);

        let seq_num = UbfU16::new(0);
        let pdu_header = create_pdu_header(seq_num);
        let metadata_pdu = create_metadata_pdu(
            &pdu_header,
            src_name.as_path(),
            dest_name.as_path(),
            file_data.len() as u64,
        );
        let packet_info = create_packet_info(&metadata_pdu, &mut buf);
        let result = dest_handler.state_machine(&mut test_user, Some(&packet_info));
        if let Err(e) = result {
            panic!("dest handler fsm error: {e}");
        }
        assert_ne!(dest_handler.state(), State::Idle);
        assert_eq!(dest_handler.step(), TransactionStep::ReceivingFileDataPdus);

        let offset = 0;
        let filedata_pdu = FileDataPdu::new_no_seg_metadata(pdu_header, offset, file_data);
        filedata_pdu
            .write_to_bytes(&mut buf)
            .expect("writing file data PDU failed");
        let packet_info = PacketInfo::new(&buf).expect("creating packet info failed");
        let result = dest_handler.state_machine(&mut test_user, Some(&packet_info));
        assert!(result.is_ok());

        let eof_pdu = create_no_error_eof(&file_data, &pdu_header, &mut buf);
        let packet_info = create_packet_info(&eof_pdu, &mut buf);
        let result = dest_handler.state_machine(&mut test_user, Some(&packet_info));
        assert!(result.is_ok());
        assert_eq!(dest_handler.state(), State::Idle);
        assert_eq!(dest_handler.step(), TransactionStep::Idle);

        assert!(Path::exists(&dest_name));
        let read_content = fs::read_to_string(&dest_name).expect("reading back string failed");
        assert_eq!(read_content, file_data_str);
        assert!(fs::remove_file(dest_name).is_ok());
    }

    #[test]
    fn test_segmented_file_transfer_not_acked() {
        let (src_name, dest_name) = init_full_filenames();
        assert!(!Path::exists(&dest_name));
        let mut rng = rand::thread_rng();
        let mut random_data = [0u8; 512];
        rng.fill(&mut random_data);
        let mut buf: [u8; 512] = [0; 512];
        let mut test_user = TestCfdpUser {
            next_expected_seq_num: 0,
            expected_full_src_name: src_name.to_string_lossy().into(),
            expected_full_dest_name: dest_name.to_string_lossy().into(),
            expected_file_size: random_data.len(),
        };

        // We treat the destination handler like it is a remote entity.
        let mut dest_handler = default_dest_handler();
        init_check(&dest_handler);

        let seq_num = UbfU16::new(0);
        let pdu_header = create_pdu_header(seq_num);
        let metadata_pdu = create_metadata_pdu(
            &pdu_header,
            src_name.as_path(),
            dest_name.as_path(),
            random_data.len() as u64,
        );
        let packet_info = create_packet_info(&metadata_pdu, &mut buf);
        let result = dest_handler.state_machine(&mut test_user, Some(&packet_info));
        if let Err(e) = result {
            panic!("dest handler fsm error: {e}");
        }
        assert_ne!(dest_handler.state(), State::Idle);
        assert_eq!(dest_handler.step(), TransactionStep::ReceivingFileDataPdus);

        // First file data PDU
        let mut offset: usize = 0;
        let segment_len = 256;
        let filedata_pdu = FileDataPdu::new_no_seg_metadata(
            pdu_header,
            offset as u64,
            &random_data[0..segment_len],
        );
        filedata_pdu
            .write_to_bytes(&mut buf)
            .expect("writing file data PDU failed");
        let packet_info = PacketInfo::new(&buf).expect("creating packet info failed");
        let result = dest_handler.state_machine(&mut test_user, Some(&packet_info));
        assert!(result.is_ok());

        // Second file data PDU
        offset += segment_len;
        let filedata_pdu = FileDataPdu::new_no_seg_metadata(
            pdu_header,
            offset as u64,
            &random_data[segment_len..],
        );
        filedata_pdu
            .write_to_bytes(&mut buf)
            .expect("writing file data PDU failed");
        let packet_info = PacketInfo::new(&buf).expect("creating packet info failed");
        let result = dest_handler.state_machine(&mut test_user, Some(&packet_info));
        assert!(result.is_ok());

        let eof_pdu = create_no_error_eof(&random_data, &pdu_header, &mut buf);
        let packet_info = create_packet_info(&eof_pdu, &mut buf);
        let result = dest_handler.state_machine(&mut test_user, Some(&packet_info));
        assert!(result.is_ok());
        assert_eq!(dest_handler.state(), State::Idle);
        assert_eq!(dest_handler.step(), TransactionStep::Idle);

        // Clean up
        assert!(Path::exists(&dest_name));
        let read_content = fs::read(&dest_name).expect("reading back string failed");
        assert_eq!(read_content, random_data);
        assert!(fs::remove_file(dest_name).is_ok());
    }
}
