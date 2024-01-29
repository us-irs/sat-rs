use spacepackets::{
    cfdp::{
        pdu::{
            file_data::SegmentMetadata,
            finished::{DeliveryCode, FileStatus},
        },
        tlv::{msg_to_user::MsgToUserTlv, WritableTlv},
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
    pub msgs_to_user: &'msgs_to_user [MsgToUserTlv<'msgs_to_user>],
}

#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct OwnedMetadataRecvdParams {
    pub id: TransactionId,
    pub source_id: UnsignedByteField,
    pub file_size: u64,
    pub src_file_name: alloc::string::String,
    pub dest_file_name: alloc::string::String,
    pub msgs_to_user: alloc::vec::Vec<alloc::vec::Vec<u8>>,
}

#[cfg(feature = "alloc")]
impl From<MetadataReceivedParams<'_, '_, '_>> for OwnedMetadataRecvdParams {
    fn from(value: MetadataReceivedParams) -> Self {
        Self::from(&value)
    }
}

#[cfg(feature = "alloc")]
impl From<&MetadataReceivedParams<'_, '_, '_>> for OwnedMetadataRecvdParams {
    fn from(value: &MetadataReceivedParams) -> Self {
        Self {
            id: value.id,
            source_id: value.source_id,
            file_size: value.file_size,
            src_file_name: value.src_file_name.into(),
            dest_file_name: value.dest_file_name.into(),
            msgs_to_user: value.msgs_to_user.iter().map(|tlv| tlv.to_vec()).collect(),
        }
    }
}

#[derive(Debug)]
pub struct FileSegmentRecvdParams<'seg_meta> {
    pub id: TransactionId,
    pub offset: u64,
    pub length: usize,
    pub segment_metadata: Option<&'seg_meta SegmentMetadata<'seg_meta>>,
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
    fn abandoned_indication(
        &mut self,
        id: &TransactionId,
        condition_code: ConditionCode,
        progress: u64,
    );
    fn eof_recvd_indication(&mut self, id: &TransactionId);
}
