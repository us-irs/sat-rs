use spacepackets::{
    cfdp::{
        pdu::{
            file_data::RecordContinuationState,
            finished::{DeliveryCode, FileStatus},
        },
        ConditionCode,
    },
    util::UnsignedByteField,
};

use super::TransactionId;

#[derive(Debug, Copy, Clone)]
pub struct TransactionFinishedParams {
    pub id: TransactionId,
    pub condition_code: ConditionCode,
    pub delivery_code: DeliveryCode,
    pub file_status: FileStatus,
}

#[derive(Debug)]
pub struct MetadataReceivedParams<'src_file, 'dest_file, 'msgs_to_user> {
    pub id: TransactionId,
    pub source_id: UnsignedByteField,
    pub file_size: u64,
    pub src_file_name: &'src_file str,
    pub dest_file_name: &'dest_file str,
    // TODO: This is pretty low-level. Is there a better way to do this?
    pub msgs_to_user: &'msgs_to_user [u8],
}

#[derive(Debug)]
pub struct FileSegmentRecvdParams<'seg_meta> {
    pub id: TransactionId,
    pub offset: u64,
    pub length: usize,
    pub rec_cont_state: Option<RecordContinuationState>,
    pub segment_metadata: Option<&'seg_meta [u8]>,
}

pub trait CfdpUser {
    fn transaction_indication(&mut self, id: &TransactionId);
    fn eof_sent_indication(&mut self, id: &TransactionId);
    fn transaction_finished_indication(&mut self, finished_params: &TransactionFinishedParams);
    fn metadata_recvd_indication(&mut self, md_recvd_params: &MetadataReceivedParams);
    fn file_segment_recvd_indication(&mut self, segment_recvd_params: &FileSegmentRecvdParams);
    // TODO: The standard does not strictly specify how the report information looks..
    fn report_indication(&mut self, id: &TransactionId);
    fn suspended_indication(&mut self, id: &TransactionId, condition_code: ConditionCode);
    fn resumed_indication(&mut self, id: &TransactionId, progress: u64);
    fn fault_indication(
        &mut self,
        id: &TransactionId,
        condition_code: ConditionCode,
        progress: u64,
    );
    fn abandoned_indication(&mut self, id: &TransactionId, condition_code: ConditionCode, progress: u64);
    fn eof_recvd_indication(&mut self, id: &TransactionId);
}
