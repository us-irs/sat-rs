use core::str::{from_utf8, Utf8Error};
use std::{
    fs::{metadata, File},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use super::{State, TransactionStep, CRC_32};
use spacepackets::{
    cfdp::{
        pdu::{
            eof::EofPdu,
            file_data::FileDataPdu,
            finished::{DeliveryCode, FileStatus, FinishedPdu},
            metadata::{MetadataGenericParams, MetadataPdu},
            CommonPduConfig, FileDirectiveType, PduError, PduHeader,
        },
        tlv::EntityIdTlv,
        ConditionCode, PduType,
    },
    util::UnsignedByteField,
};
use thiserror::Error;

pub struct DestinationHandler {
    id: UnsignedByteField,
    step: TransactionStep,
    state: State,
    pdu_conf: CommonPduConfig,
    transaction_params: TransactionParams,
    packets_to_send_ctx: PacketsToSendContext,
}

#[derive(Debug, Default)]
struct PacketsToSendContext {
    packet_available: bool,
    directive: Option<FileDirectiveType>,
}

struct TransactionParams {
    metadata_params: MetadataGenericParams,
    src_file_name: [u8; u8::MAX as usize],
    src_file_name_len: usize,
    dest_file_name: [u8; u8::MAX as usize],
    dest_file_name_len: usize,
    dest_path_buf: PathBuf,
    condition_code: ConditionCode,
    delivery_code: DeliveryCode,
    file_status: FileStatus,
    cksum_buf: [u8; 1024],
}

impl Default for TransactionParams {
    fn default() -> Self {
        Self {
            metadata_params: Default::default(),
            src_file_name: [0; u8::MAX as usize],
            src_file_name_len: Default::default(),
            dest_file_name: [0; u8::MAX as usize],
            dest_file_name_len: Default::default(),
            dest_path_buf: Default::default(),
            condition_code: ConditionCode::NoError,
            delivery_code: DeliveryCode::Incomplete,
            file_status: FileStatus::Unreported,
            cksum_buf: [0; 1024],
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
            pdu_conf: Default::default(),
            transaction_params: Default::default(),
            packets_to_send_ctx: Default::default(),
        }
    }

    pub fn state_machine(&mut self) -> Result<(), DestError> {
        match self.state {
            State::Idle => todo!(),
            State::BusyClass1Nacked => self.fsm_nacked(),
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
                let pdu_header = PduHeader::new_no_file_data(self.pdu_conf, 0);
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
        self.transaction_params.src_file_name[..src_name.len_value()]
            .copy_from_slice(src_name.value().unwrap());
        self.transaction_params.src_file_name_len = src_name.len_value();
        let dest_name = metadata_pdu.dest_file_name();
        if dest_name.is_empty() {
            return Err(DestError::EmptyDestFileField);
        }
        self.transaction_params.dest_file_name[..dest_name.len_value()]
            .copy_from_slice(dest_name.value().unwrap());
        self.transaction_params.dest_file_name_len = dest_name.len_value();
        Ok(())
    }

    pub fn handle_file_data(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
        if self.state == State::Idle || self.step != TransactionStep::ReceivingFileDataPdus {
            return Err(DestError::WrongStateForFileDataAndEof);
        }
        let fd_pdu = FileDataPdu::from_bytes(raw_packet)?;
        let mut dest_file = File::options()
            .write(true)
            .open(&self.transaction_params.dest_path_buf)?;
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

    pub fn handle_prompt_pdu(&mut self, raw_packet: &[u8]) -> Result<(), DestError> {
        todo!();
        Ok(())
    }

    fn checksum_check(&mut self, expected_checksum: u32) -> Result<bool, DestError> {
        let mut digest = CRC_32.digest();
        let file_to_check = File::open(&self.transaction_params.dest_path_buf)?;
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

    fn fsm_nacked(&mut self) -> Result<(), DestError> {
        if self.step == TransactionStep::Idle {}
        if self.step == TransactionStep::TransactionStart {
            self.transaction_start()?;
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

    fn transaction_start(&mut self) -> Result<(), DestError> {
        let dest_path = Path::new(from_utf8(
            &self.transaction_params.dest_file_name[..self.transaction_params.dest_file_name_len],
        )?);

        self.transaction_params.dest_path_buf = dest_path.to_path_buf();

        let metadata = metadata(dest_path)?;
        if metadata.is_dir() {
            // Create new destination path by concatenating the last part of the source source
            // name and the destination folder. For example, for a source file of /tmp/hello.txt
            // and a destination name of /home/test, the resulting file name should be
            // /home/test/hello.txt
            let source_path = Path::new(from_utf8(
                &self.transaction_params.src_file_name[..self.transaction_params.src_file_name_len],
            )?);

            let source_name = source_path.file_name();
            if source_name.is_none() {
                return Err(DestError::PathConcatError);
            }
            let source_name = source_name.unwrap();
            self.transaction_params.dest_path_buf.push(source_name);
        }
        // This function does exactly what we require: Create a new file if it does not exist yet
        // and trucate an existing one.
        File::create(&self.transaction_params.dest_path_buf)?;
        Ok(())
    }

    fn transfer_completion(&mut self) -> Result<(), DestError> {
        if self.transaction_params.metadata_params.closure_requested {
            self.prepare_finished_pdu()?;
        }
        todo!("user indication");
        Ok(())
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
    use spacepackets::util::UnsignedByteFieldU8;

    use super::*;

    #[test]
    fn test_basic() {
        let test_id = UnsignedByteFieldU8::new(1);
        let dest_handler = DestinationHandler::new(test_id);
        assert_eq!(dest_handler.state(), State::Idle);
        assert_eq!(dest_handler.step(), TransactionStep::Idle);
    }
}
