use core::str::{from_utf8, Utf8Error};
use std::{
    fs::{metadata, File},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use super::{
    user::{CfdpUser, MetadataReceivedParams},
    State, TransactionId, TransactionStep, CRC_32,
};
use smallvec::SmallVec;
use spacepackets::{
    cfdp::{
        pdu::{
            eof::EofPdu,
            file_data::FileDataPdu,
            finished::{DeliveryCode, FileStatus, FinishedPdu},
            metadata::{MetadataGenericParams, MetadataPdu},
            CommonPduConfig, FileDirectiveType, PduError, PduHeader,
        },
        tlv::{msg_to_user::MsgToUserTlv, EntityIdTlv, TlvType},
        ConditionCode, PduType,
    },
    util::UnsignedByteField,
};
use thiserror::Error;

pub struct DestinationHandler {
    id: UnsignedByteField,
    step: TransactionStep,
    state: State,
    transaction_params: TransactionParams,
    packets_to_send_ctx: PacketsToSendContext,
    //cfdp_user: Box<dyn CfdpUser>,
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
struct TransactionParams {
    transaction_id: Option<TransactionId>,
    metadata_params: MetadataGenericParams,
    pdu_conf: CommonPduConfig,
    file_properties: FileProperties,
    condition_code: ConditionCode,
    delivery_code: DeliveryCode,
    file_status: FileStatus,
    //msgs_to_user: Vec<MsgToUserTlv<'static>>,
    cksum_buf: [u8; 1024],
    msgs_to_user_size: usize,
    msgs_to_user_buf: [u8; 1024],
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

impl Default for TransactionParams {
    fn default() -> Self {
        Self {
            transaction_id: None,
            metadata_params: Default::default(),
            pdu_conf: Default::default(),
            file_properties: Default::default(),
            condition_code: ConditionCode::NoError,
            delivery_code: DeliveryCode::Incomplete,
            file_status: FileStatus::Unreported,
            cksum_buf: [0; 1024],
            msgs_to_user_size: 0,
            msgs_to_user_buf: [0; 1024],
        }
    }
}

impl TransactionParams {
    fn reset(&mut self) {
        self.condition_code = ConditionCode::NoError;
        self.delivery_code = DeliveryCode::Incomplete;
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
}

impl DestinationHandler {
    pub fn new(id: impl Into<UnsignedByteField>) -> Self {
        Self {
            id: id.into(),
            step: TransactionStep::Idle,
            state: State::Idle,
            transaction_params: Default::default(),
            packets_to_send_ctx: Default::default(),
            //cfdp_user,
        }
    }

    pub fn state_machine(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<(), DestError> {
        match self.state {
            State::Idle => todo!(),
            State::BusyClass1Nacked => self.fsm_nacked(cfdp_user),
            State::BusyClass2Acked => todo!(),
        }
    }

    pub fn insert_packet(
        &mut self,
        pdu_type: PduType,
        pdu_directive: Option<FileDirectiveType>,
        raw_packet: &[u8],
    ) -> Result<(), DestError> {
        match pdu_type {
            PduType::FileDirective => {
                if pdu_directive.is_none() {
                    return Err(DestError::DirectiveExpected);
                }
                self.handle_file_directive(pdu_directive.unwrap(), raw_packet)
            }
            PduType::FileData => self.handle_file_data(raw_packet),
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
                let pdu_header = PduHeader::new_no_file_data(self.transaction_params.pdu_conf, 0);
                let finished_pdu = if self.transaction_params.condition_code
                    == ConditionCode::NoError
                    || self.transaction_params.condition_code
                        == ConditionCode::UnsupportedChecksumType
                {
                    FinishedPdu::new_default(
                        pdu_header,
                        self.transaction_params.delivery_code,
                        self.transaction_params.file_status,
                    )
                } else {
                    // TODO: Are there cases where this ID is actually the source entity ID?
                    let entity_id = EntityIdTlv::new(self.id);
                    FinishedPdu::new_with_error(
                        pdu_header,
                        self.transaction_params.condition_code,
                        self.transaction_params.delivery_code,
                        self.transaction_params.file_status,
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
        pdu_directive: FileDirectiveType,
        raw_packet: &[u8],
    ) -> Result<(), DestError> {
        match pdu_directive {
            FileDirectiveType::EofPdu => self.handle_eof_pdu(raw_packet)?,
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
        let metadata_pdu = MetadataPdu::from_bytes(raw_packet)?;
        self.transaction_params.reset();
        self.transaction_params.metadata_params = *metadata_pdu.metadata_params();
        let src_name = metadata_pdu.src_file_name();
        if src_name.is_empty() {
            return Err(DestError::EmptySrcFileField);
        }
        self.transaction_params.file_properties.src_file_name[..src_name.len_value()]
            .copy_from_slice(src_name.value());
        self.transaction_params.file_properties.src_file_name_len = src_name.len_value();
        let dest_name = metadata_pdu.dest_file_name();
        if dest_name.is_empty() {
            return Err(DestError::EmptyDestFileField);
        }
        self.transaction_params.file_properties.dest_file_name[..dest_name.len_value()]
            .copy_from_slice(dest_name.value());
        self.transaction_params.file_properties.dest_file_name_len = dest_name.len_value();
        self.transaction_params.pdu_conf = *metadata_pdu.pdu_header().common_pdu_conf();
        self.transaction_params.msgs_to_user_size = 0;
        if metadata_pdu.options().is_some() {
            for option_tlv in metadata_pdu.options_iter().unwrap() {
                if option_tlv.is_standard_tlv()
                    && option_tlv.tlv_type().unwrap() == TlvType::MsgToUser
                {
                    self.transaction_params
                        .msgs_to_user_buf
                        .copy_from_slice(option_tlv.raw_data().unwrap());
                    self.transaction_params.msgs_to_user_size += option_tlv.len_full();
                }
            }
        }
        Ok(())
    }

    pub fn handle_file_data(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
        if self.state == State::Idle || self.step != TransactionStep::ReceivingFileDataPdus {
            return Err(DestError::WrongStateForFileDataAndEof);
        }
        let fd_pdu = FileDataPdu::from_bytes(raw_packet)?;
        let mut dest_file = File::options()
            .write(true)
            .open(&self.transaction_params.file_properties.dest_path_buf)?;
        dest_file.seek(SeekFrom::Start(fd_pdu.offset()))?;
        dest_file.write_all(fd_pdu.file_data())?;
        Ok(())
    }

    pub fn handle_eof_pdu(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
        if self.state == State::Idle || self.step != TransactionStep::ReceivingFileDataPdus {
            return Err(DestError::WrongStateForFileDataAndEof);
        }
        let eof_pdu = EofPdu::from_bytes(raw_packet)?;
        let checksum = eof_pdu.file_checksum();
        // For a standard disk based file system, which is assumed to be used for now, the file
        // will always be retained. This might change in the future.
        self.transaction_params.file_status = FileStatus::Retained;
        if self.checksum_check(checksum)? {
            self.transaction_params.condition_code = ConditionCode::NoError;
            self.transaction_params.delivery_code = DeliveryCode::Complete;
        } else {
            self.transaction_params.condition_code = ConditionCode::FileChecksumFailure;
        }
        if self.state == State::BusyClass1Nacked {
            self.step = TransactionStep::TransferCompletion;
        } else {
            self.step = TransactionStep::SendingAckPdu;
        }
        Ok(())
    }

    pub fn handle_prompt_pdu(&mut self, _raw_packet: &[u8]) -> Result<(), DestError> {
        todo!();
    }

    fn checksum_check(&mut self, expected_checksum: u32) -> Result<bool, DestError> {
        let mut digest = CRC_32.digest();
        let file_to_check = File::open(&self.transaction_params.file_properties.dest_path_buf)?;
        let mut buf_reader = BufReader::new(file_to_check);
        loop {
            let bytes_read = buf_reader.read(&mut self.transaction_params.cksum_buf)?;
            if bytes_read == 0 {
                break;
            }
            digest.update(&self.transaction_params.cksum_buf[0..bytes_read]);
        }
        if digest.finalize() == expected_checksum {
            return Ok(true);
        }
        Ok(false)
    }

    fn fsm_nacked(&mut self, cfdp_user: &mut impl CfdpUser) -> Result<(), DestError> {
        if self.step == TransactionStep::Idle {}
        if self.step == TransactionStep::TransactionStart {
            self.transaction_start(cfdp_user)?;
        }
        if self.step == TransactionStep::ReceivingFileDataPdus {
            todo!("advance the fsm if everything is finished")
        }
        if self.step == TransactionStep::TransferCompletion {
            self.transfer_completion()?;
        }
        if self.step == TransactionStep::SendingAckPdu {
            todo!();
        }
        if self.step == TransactionStep::SendingFinishedPdu {
            self.reset();
            return Ok(());
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
            &self.transaction_params.file_properties.dest_file_name
                [..self.transaction_params.file_properties.dest_file_name_len],
        )?;
        let dest_path = Path::new(dest_name);
        self.transaction_params.file_properties.dest_path_buf = dest_path.to_path_buf();
        let source_id = self.transaction_params.pdu_conf.source_id();
        let id = TransactionId::new(
            source_id,
            self.transaction_params.pdu_conf.transaction_seq_num,
        );
        let src_name = from_utf8(
            &self.transaction_params.file_properties.src_file_name
                [0..self.transaction_params.file_properties.src_file_name_len],
        )?;
        let mut msgs_to_user = SmallVec::<[MsgToUserTlv<'_>; 16]>::new();
        let mut num_msgs_to_user = 0;
        if self.transaction_params.msgs_to_user_size > 0 {
            let mut index = 0;
            while index < self.transaction_params.msgs_to_user_size {
                // This should never panic as the validity of the options was checked beforehand.
                let msgs_to_user_tlv =
                    MsgToUserTlv::from_bytes(&self.transaction_params.msgs_to_user_buf[index..])
                        .expect("message to user creation failed unexpectedly");
                msgs_to_user.push(msgs_to_user_tlv);
                index += msgs_to_user_tlv.len_full();
                num_msgs_to_user += 1;
            }
        }
        let metadata_recvd_params = MetadataReceivedParams {
            id,
            source_id,
            file_size: self.transaction_params.metadata_params.file_size,
            src_file_name: src_name,
            dest_file_name: dest_name,
            msgs_to_user: &msgs_to_user[..num_msgs_to_user],
        };
        self.transaction_params.transaction_id = Some(id);
        cfdp_user.metadata_recvd_indication(&metadata_recvd_params);

        let metadata = metadata(dest_path)?;
        if metadata.is_dir() {
            // Create new destination path by concatenating the last part of the source source
            // name and the destination folder. For example, for a source file of /tmp/hello.txt
            // and a destination name of /home/test, the resulting file name should be
            // /home/test/hello.txt
            let source_path = Path::new(from_utf8(
                &self.transaction_params.file_properties.src_file_name
                    [..self.transaction_params.file_properties.src_file_name_len],
            )?);

            let source_name = source_path.file_name();
            if source_name.is_none() {
                return Err(DestError::PathConcatError);
            }
            let source_name = source_name.unwrap();
            self.transaction_params
                .file_properties
                .dest_path_buf
                .push(source_name);
        }
        // This function does exactly what we require: Create a new file if it does not exist yet
        // and trucate an existing one.
        File::create(&self.transaction_params.file_properties.dest_path_buf)?;
        Ok(())
    }

    fn transfer_completion(&mut self) -> Result<(), DestError> {
        // This function should never be called with metadata parameters not set
        if self.transaction_params.metadata_params.closure_requested {
            self.prepare_finished_pdu()?;
        }
        todo!("user indication");
    }

    fn reset(&mut self) {
        self.step = TransactionStep::Idle;
        self.state = State::Idle;
        self.packets_to_send_ctx.packet_available = false;
        self.transaction_params.reset();
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
    use spacepackets::util::UnsignedByteFieldU16;

    use super::*;

    const LOCAL_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(1);
    const REMOTE_ID: UnsignedByteFieldU16 = UnsignedByteFieldU16::new(2);

    #[derive(Default)]
    struct TestCfdpUser {
        next_expected_seq_num: u64,
    }

    impl TestCfdpUser {
        fn generic_id_check(&self, id: &crate::cfdp::TransactionId) {
            assert_eq!(id.source_id, REMOTE_ID.into());
            assert_eq!(id.seq_num().value(), self.next_expected_seq_num);
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
            _md_recvd_params: &crate::cfdp::user::MetadataReceivedParams,
        ) {
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
        }

        fn resumed_indication(&mut self, _id: &crate::cfdp::TransactionId, _progresss: u64) {}

        fn fault_indication(
            &mut self,
            _id: &crate::cfdp::TransactionId,
            _condition_code: ConditionCode,
            _progress: u64,
        ) {
        }

        fn abandoned_indication(
            &mut self,
            _id: &crate::cfdp::TransactionId,
            _condition_code: ConditionCode,
            _progress: u64,
        ) {
        }

        fn eof_recvd_indication(&mut self, _id: &crate::cfdp::TransactionId) {}
    }

    fn init_check(handler: &DestinationHandler) {
        assert_eq!(handler.state(), State::Idle);
        assert_eq!(handler.step(), TransactionStep::Idle);
    }

    #[test]
    fn test_basic() {
        let dest_handler = DestinationHandler::new(LOCAL_ID);
        init_check(&dest_handler);
    }

    #[test]
    fn test_empty_file_transfer() {
        let test_user = TestCfdpUser::default();
        let mut dest_handler = DestinationHandler::new(LOCAL_ID);
        init_check(&dest_handler);

        // TODO: Create Metadata PDU and EOF PDU for empty file transfer.
        //dest_handler.insert_packet(pdu_type, pdu_directive, raw_packet)
    }
}
