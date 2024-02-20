//! This module contains the implementation of the CFDP high level classes as specified in the
//! CCSDS 727.0-B-5.
use core::{cell::RefCell, fmt::Debug, hash::Hash};

use crc::{Crc, CRC_32_CKSUM};
use hashbrown::HashMap;
use spacepackets::{
    cfdp::{
        pdu::{FileDirectiveType, PduError, PduHeader},
        ChecksumType, ConditionCode, FaultHandlerCode, PduType, TransmissionMode,
    },
    util::{UnsignedByteField, UnsignedEnum},
};

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
pub mod dest;
#[cfg(feature = "alloc")]
pub mod filestore;
#[cfg(feature = "std")]
pub mod source;
pub mod user;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityType {
    Sending,
    Receiving,
}

pub enum TimerContext {
    CheckLimit {
        local_id: UnsignedByteField,
        remote_id: UnsignedByteField,
        entity_type: EntityType,
    },
    NakActivity {
        expiry_time_seconds: f32,
    },
    PositiveAck {
        expiry_time_seconds: f32,
    },
}

/// Generic abstraction for a check timer which is used by 3 mechanisms of the CFDP protocol.
///
/// ## 1. Check limit handling
///
/// The first mechanism is the check limit handling for unacknowledged transfers as specified
/// in 4.6.3.2 and 4.6.3.3 of the CFDP standard.
/// For this mechanism, the timer has different functionality depending on whether
/// the using entity is the sending entity or the receiving entity for the unacknowledged
/// transmission mode.
///
/// For the sending entity, this timer determines the expiry period for declaring a check limit
/// fault after sending an EOF PDU with requested closure. This allows a timeout of the transfer.
/// Also see 4.6.3.2 of the CFDP standard.
///
/// For the receiving entity, this timer determines the expiry period for incrementing a check
/// counter after an EOF PDU is received for an incomplete file transfer. This allows out-of-order
/// reception of file data PDUs and EOF PDUs. Also see 4.6.3.3 of the CFDP standard.
///
/// ## 2. NAK activity limit
///
/// The timer will be used to perform the NAK activity check as specified in 4.6.4.7 of the CFDP
/// standard. The expiration period will be provided by the NAK timer expiration limit of the
/// remote entity configuration.
///
/// ## 3. Positive ACK procedures
///
/// The timer will be used to perform the Positive Acknowledgement Procedures as  specified in
/// 4.7. 1of the CFDP standard. The expiration period will be provided by the Positive ACK timer
/// interval of the remote entity configuration.
pub trait CheckTimer: Debug {
    fn has_expired(&self) -> bool;
    fn reset(&mut self);
}

/// A generic trait which allows CFDP entities to create check timers which are required to
/// implement special procedures in unacknowledged transmission mode, as specified in 4.6.3.2
/// and 4.6.3.3. The [CheckTimer] documentation provides more information about the purpose of the
/// check timer in the context of CFDP.
///
/// This trait also allows the creation of different check timers depending on context and purpose
/// of the timer, the runtime environment (e.g. standard clock timer vs. timer using a RTC) or
/// other factors.
#[cfg(feature = "alloc")]
pub trait CheckTimerCreator {
    fn get_check_timer_provider(&self, timer_context: TimerContext) -> Box<dyn CheckTimer>;
}

/// Simple implementation of the [CheckTimerCreator] trait assuming a standard runtime.
/// It also assumes that a second accuracy of the check timer period is sufficient.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct StdCheckTimer {
    expiry_time_seconds: u64,
    start_time: std::time::Instant,
}

#[cfg(feature = "std")]
impl StdCheckTimer {
    pub fn new(expiry_time_seconds: u64) -> Self {
        Self {
            expiry_time_seconds,
            start_time: std::time::Instant::now(),
        }
    }
}

#[cfg(feature = "std")]
impl CheckTimer for StdCheckTimer {
    fn has_expired(&self) -> bool {
        let elapsed_time = self.start_time.elapsed();
        if elapsed_time.as_secs() > self.expiry_time_seconds {
            return true;
        }
        false
    }

    fn reset(&mut self) {
        self.start_time = std::time::Instant::now();
    }
}

/// This structure models the remote entity configuration information as specified in chapter 8.3
/// of the CFDP standard.

/// Some of the fields which were not considered necessary for the Rust implementation
/// were omitted. Some other fields which are not contained inside the standard but are considered
/// necessary for the Rust implementation are included.
///
/// ## Notes on Positive Acknowledgment Procedures
///
/// The `positive_ack_timer_interval_seconds` and `positive_ack_timer_expiration_limit` will
/// be used for positive acknowledgement procedures as specified in CFDP chapter 4.7. The sending
/// entity will start the timer for any PDUs where an acknowledgment is required (e.g. EOF PDU).
/// Once the expected ACK response has not been received for that interval, as counter will be
/// incremented and the timer will be reset. Once the counter exceeds the
/// `positive_ack_timer_expiration_limit`, a Positive ACK Limit Reached fault will be declared.
///
/// ## Notes on Deferred Lost Segment Procedures
///
/// This procedure will be active if an EOF (No Error) PDU is received in acknowledged mode. After
/// issuing the NAK sequence which has the whole file scope, a timer will be started. The timer is
/// reset when missing segments or missing metadata is received. The timer will be deactivated if
/// all missing data is received. If the timer expires, a new NAK sequence will be issued and a
/// counter will be incremented, which can lead to a NAK Limit Reached fault being declared.
///
/// ## Fields
///
/// * `entity_id` - The ID of the remote entity.
/// * `max_packet_len` - This determines of all PDUs generated for that remote entity in addition
///    to the `max_file_segment_len` attribute which also determines the size of file data PDUs.
/// * `max_file_segment_len` The maximum file segment length which determines the maximum size
///   of file data PDUs in addition to the `max_packet_len` attribute. If this field is set
///   to None, the maximum file segment length will be derived from the maximum packet length.
///   If this has some value which is smaller than the segment value derived from
///   `max_packet_len`, this value will be picked.
/// * `closure_requested_by_default` - If the closure requested field is not supplied as part of
///    the Put Request, it will be determined from this field in the remote configuration.
/// * `crc_on_transmission_by_default` - If the CRC option is not supplied as part of the Put
///    Request, it will be determined from this field in the remote configuration.
/// * `default_transmission_mode` - If the transmission mode is not supplied as part of the
///   Put Request, it will be determined from this field in the remote configuration.
/// * `disposition_on_cancellation` - Determines whether an incomplete received file is discard on
///   transaction cancellation. Defaults to False.
/// * `default_crc_type` - Default checksum type used to calculate for all file transmissions to
///   this remote entity.
/// * `check_limit` - This timer determines the expiry period for incrementing a check counter
///   after an EOF PDU is received for an incomplete file transfer. This allows out-of-order
///   reception of file data PDUs and EOF PDUs. Also see 4.6.3.3 of the CFDP standard. Defaults to
///   2, so the check limit timer may expire twice.
/// * `positive_ack_timer_interval_seconds`- See the notes on the Positive Acknowledgment
///    Procedures inside the class documentation. Expected as floating point seconds. Defaults to
///    10 seconds.
/// * `positive_ack_timer_expiration_limit` - See the notes on the Positive Acknowledgment
///    Procedures inside the class documentation. Defaults to 2, so the timer may expire twice.
/// * `immediate_nak_mode` - Specifies whether a NAK sequence should be issued immediately when a
///    file data gap or lost metadata is detected in the acknowledged mode. Defaults to True.
/// * `nak_timer_interval_seconds` -  See the notes on the Deferred Lost Segment Procedure inside
///    the class documentation. Expected as floating point seconds. Defaults to 10 seconds.
/// * `nak_timer_expiration_limit` - See the notes on the Deferred Lost Segment Procedure inside
///    the class documentation. Defaults to 2, so the timer may expire two times.
#[derive(Debug, Copy, Clone)]
pub struct RemoteEntityConfig {
    pub entity_id: UnsignedByteField,
    pub max_packet_len: usize,
    pub max_file_segment_len: usize,
    pub closure_requested_by_default: bool,
    pub crc_on_transmission_by_default: bool,
    pub default_transmission_mode: TransmissionMode,
    pub default_crc_type: ChecksumType,
    pub positive_ack_timer_interval_seconds: f32,
    pub positive_ack_timer_expiration_limit: u32,
    pub check_limit: u32,
    pub disposition_on_cancellation: bool,
    pub immediate_nak_mode: bool,
    pub nak_timer_interval_seconds: f32,
    pub nak_timer_expiration_limit: u32,
}

impl RemoteEntityConfig {
    pub fn new_with_default_values(
        entity_id: UnsignedByteField,
        max_file_segment_len: usize,
        max_packet_len: usize,
        closure_requested_by_default: bool,
        crc_on_transmission_by_default: bool,
        default_transmission_mode: TransmissionMode,
        default_crc_type: ChecksumType,
    ) -> Self {
        Self {
            entity_id,
            max_file_segment_len,
            max_packet_len,
            closure_requested_by_default,
            crc_on_transmission_by_default,
            default_transmission_mode,
            default_crc_type,
            check_limit: 2,
            positive_ack_timer_interval_seconds: 10.0,
            positive_ack_timer_expiration_limit: 2,
            disposition_on_cancellation: false,
            immediate_nak_mode: true,
            nak_timer_interval_seconds: 10.0,
            nak_timer_expiration_limit: 2,
        }
    }
}

pub trait RemoteEntityConfigProvider {
    /// Retrieve the remote entity configuration for the given remote ID.
    fn get_remote_config(&self, remote_id: u64) -> Option<&RemoteEntityConfig>;
    fn get_remote_config_mut(&mut self, remote_id: u64) -> Option<&mut RemoteEntityConfig>;
    /// Add a new remote configuration. Return [true] if the configuration was
    /// inserted successfully, and [false] if a configuration already exists.
    fn add_config(&mut self, cfg: &RemoteEntityConfig) -> bool;
    /// Remote a configuration. Returns [true] if the configuration was removed successfully,
    /// and [false] if no configuration exists for the given remote ID.
    fn remove_config(&mut self, remote_id: u64) -> bool;
}

#[cfg(feature = "std")]
#[derive(Default)]
pub struct StdRemoteEntityConfigProvider {
    remote_cfg_table: HashMap<u64, RemoteEntityConfig>,
}

#[cfg(feature = "std")]
impl RemoteEntityConfigProvider for StdRemoteEntityConfigProvider {
    fn get_remote_config(&self, remote_id: u64) -> Option<&RemoteEntityConfig> {
        self.remote_cfg_table.get(&remote_id)
    }
    fn get_remote_config_mut(&mut self, remote_id: u64) -> Option<&mut RemoteEntityConfig> {
        self.remote_cfg_table.get_mut(&remote_id)
    }
    fn add_config(&mut self, cfg: &RemoteEntityConfig) -> bool {
        self.remote_cfg_table
            .insert(cfg.entity_id.value(), *cfg)
            .is_some()
    }
    fn remove_config(&mut self, remote_id: u64) -> bool {
        self.remote_cfg_table.remove(&remote_id).is_some()
    }
}

/// This trait introduces some callbacks which will be called when a particular CFDP fault
/// handler is called.
///
/// It is passed into the CFDP handlers as part of the [DefaultFaultHandler] and the local entity
/// configuration and provides a way to specify custom user error handlers. This allows to
/// implement some CFDP features like fault handler logging, which would not be possible
/// generically otherwise.
///
/// For each error reported by the [DefaultFaultHandler], the appropriate fault handler callback
/// will be called depending on the [FaultHandlerCode].
pub trait UserFaultHandler {
    fn notice_of_suspension_cb(
        &mut self,
        transaction_id: TransactionId,
        cond: ConditionCode,
        progress: u64,
    );

    fn notice_of_cancellation_cb(
        &mut self,
        transaction_id: TransactionId,
        cond: ConditionCode,
        progress: u64,
    );

    fn abandoned_cb(&mut self, transaction_id: TransactionId, cond: ConditionCode, progress: u64);

    fn ignore_cb(&mut self, transaction_id: TransactionId, cond: ConditionCode, progress: u64);
}

/// This structure is used to implement the fault handling as specified in chapter 4.8 of the CFDP
/// standard.
///
/// It does so by mapping each applicable [spacepackets::cfdp::ConditionCode] to a fault handler
/// which is denoted by the four [spacepackets::cfdp::FaultHandlerCode]s. This code is used
/// to select the error handling inside the CFDP handler itself in addition to dispatching to a
/// user-provided callback function provided by the [UserFaultHandler].
///
/// Some note on the provided default settings:
///
/// - Checksum failures will be ignored by default. This is because for unacknowledged transfers,
///   cancelling the transfer immediately would interfere with the check limit mechanism specified
///   in chapter 4.6.3.3.
/// - Unsupported checksum types will also be ignored by default. Even if the checksum type is
///   not supported the file transfer might still have worked properly.
///
/// For all other faults, the default fault handling operation will be to cancel the transaction.
/// These defaults can be overriden by using the [Self::set_fault_handler] method.
/// Please note that in any case, fault handler overrides can be specified by the sending CFDP
/// entity.
pub struct DefaultFaultHandler {
    handler_array: [FaultHandlerCode; 10],
    // Could also change the user fault handler trait to have non mutable methods, but that limits
    // flexbility on the user side..
    user_fault_handler: RefCell<Box<dyn UserFaultHandler + Send>>,
}

impl DefaultFaultHandler {
    fn condition_code_to_array_index(conditon_code: ConditionCode) -> Option<usize> {
        Some(match conditon_code {
            ConditionCode::PositiveAckLimitReached => 0,
            ConditionCode::KeepAliveLimitReached => 1,
            ConditionCode::InvalidTransmissionMode => 2,
            ConditionCode::FilestoreRejection => 3,
            ConditionCode::FileChecksumFailure => 4,
            ConditionCode::FileSizeError => 5,
            ConditionCode::NakLimitReached => 6,
            ConditionCode::InactivityDetected => 7,
            ConditionCode::CheckLimitReached => 8,
            ConditionCode::UnsupportedChecksumType => 9,
            _ => return None,
        })
    }

    pub fn set_fault_handler(
        &mut self,
        condition_code: ConditionCode,
        fault_handler: FaultHandlerCode,
    ) {
        let array_idx = Self::condition_code_to_array_index(condition_code);
        if array_idx.is_none() {
            return;
        }
        self.handler_array[array_idx.unwrap()] = fault_handler;
    }

    pub fn new(user_fault_handler: Box<dyn UserFaultHandler + Send>) -> Self {
        let mut init_array = [FaultHandlerCode::NoticeOfCancellation; 10];
        init_array
            [Self::condition_code_to_array_index(ConditionCode::FileChecksumFailure).unwrap()] =
            FaultHandlerCode::IgnoreError;
        init_array[Self::condition_code_to_array_index(ConditionCode::UnsupportedChecksumType)
            .unwrap()] = FaultHandlerCode::IgnoreError;
        Self {
            handler_array: init_array,
            user_fault_handler: RefCell::new(user_fault_handler),
        }
    }

    pub fn get_fault_handler(&self, condition_code: ConditionCode) -> FaultHandlerCode {
        let array_idx = Self::condition_code_to_array_index(condition_code);
        if array_idx.is_none() {
            return FaultHandlerCode::IgnoreError;
        }
        self.handler_array[array_idx.unwrap()]
    }

    pub fn report_fault(
        &self,
        transaction_id: TransactionId,
        condition: ConditionCode,
        progress: u64,
    ) -> FaultHandlerCode {
        let array_idx = Self::condition_code_to_array_index(condition);
        if array_idx.is_none() {
            return FaultHandlerCode::IgnoreError;
        }
        let fh_code = self.handler_array[array_idx.unwrap()];
        let mut handler_mut = self.user_fault_handler.borrow_mut();
        match fh_code {
            FaultHandlerCode::NoticeOfCancellation => {
                handler_mut.notice_of_cancellation_cb(transaction_id, condition, progress);
            }
            FaultHandlerCode::NoticeOfSuspension => {
                handler_mut.notice_of_suspension_cb(transaction_id, condition, progress);
            }
            FaultHandlerCode::IgnoreError => {
                handler_mut.ignore_cb(transaction_id, condition, progress);
            }
            FaultHandlerCode::AbandonTransaction => {
                handler_mut.abandoned_cb(transaction_id, condition, progress);
            }
        }
        fh_code
    }
}

pub struct IndicationConfig {
    pub eof_sent: bool,
    pub eof_recv: bool,
    pub file_segment_recv: bool,
    pub transaction_finished: bool,
    pub suspended: bool,
    pub resumed: bool,
}

impl Default for IndicationConfig {
    fn default() -> Self {
        Self {
            eof_sent: true,
            eof_recv: true,
            file_segment_recv: true,
            transaction_finished: true,
            suspended: true,
            resumed: true,
        }
    }
}

pub struct LocalEntityConfig {
    pub id: UnsignedByteField,
    pub indication_cfg: IndicationConfig,
    pub default_fault_handler: DefaultFaultHandler,
}

/// The CFDP transaction ID of a CFDP transaction consists of the source entity ID and the sequence
/// number of that transfer which is also determined by the CFDP source entity.
#[derive(Debug, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TransactionId {
    source_id: UnsignedByteField,
    seq_num: UnsignedByteField,
}

impl TransactionId {
    pub fn new(source_id: UnsignedByteField, seq_num: UnsignedByteField) -> Self {
        Self { source_id, seq_num }
    }

    pub fn source_id(&self) -> &UnsignedByteField {
        &self.source_id
    }

    pub fn seq_num(&self) -> &UnsignedByteField {
        &self.seq_num
    }
}

impl Hash for TransactionId {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.source_id.value().hash(state);
        self.seq_num.value().hash(state);
    }
}

impl PartialEq for TransactionId {
    fn eq(&self, other: &Self) -> bool {
        self.source_id.value() == other.source_id.value()
            && self.seq_num.value() == other.seq_num.value()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TransactionStep {
    Idle = 0,
    TransactionStart = 1,
    ReceivingFileDataPdus = 2,
    ReceivingFileDataPdusWithCheckLimitHandling = 3,
    SendingAckPdu = 4,
    TransferCompletion = 5,
    SendingFinishedPdu = 6,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum State {
    Idle = 0,
    Busy = 1,
    Suspended = 2,
}

pub const CRC_32: Crc<u32> = Crc::<u32>::new(&CRC_32_CKSUM);

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PacketTarget {
    SourceEntity,
    DestEntity,
}

/// This is a helper struct which contains base information about a particular PDU packet.
/// This is also necessary information for CFDP packet routing. For example, some packet types
/// like file data PDUs can only be used by CFDP source entities.
pub struct PacketInfo<'raw_packet> {
    pdu_type: PduType,
    pdu_directive: Option<FileDirectiveType>,
    target: PacketTarget,
    raw_packet: &'raw_packet [u8],
}

impl<'raw> PacketInfo<'raw> {
    pub fn new(raw_packet: &'raw [u8]) -> Result<Self, PduError> {
        let (pdu_header, header_len) = PduHeader::from_bytes(raw_packet)?;
        if pdu_header.pdu_type() == PduType::FileData {
            return Ok(Self {
                pdu_type: pdu_header.pdu_type(),
                pdu_directive: None,
                target: PacketTarget::DestEntity,
                raw_packet,
            });
        }
        if pdu_header.pdu_datafield_len() < 1 {
            return Err(PduError::FormatError);
        }
        // Route depending on PDU type and directive type if applicable. Retrieve directive type
        // from the raw stream for better performance (with sanity and directive code check).
        // The routing is based on section 4.5 of the CFDP standard which specifies the PDU forwarding
        // procedure.
        let directive = FileDirectiveType::try_from(raw_packet[header_len]).map_err(|_| {
            PduError::InvalidDirectiveType {
                found: raw_packet[header_len],
                expected: None,
            }
        })?;
        let packet_target = match directive {
            // Section c) of 4.5.3: These PDUs should always be targeted towards the file sender a.k.a.
            // the source handler
            FileDirectiveType::NakPdu
            | FileDirectiveType::FinishedPdu
            | FileDirectiveType::KeepAlivePdu => PacketTarget::SourceEntity,
            // Section b) of 4.5.3: These PDUs should always be targeted towards the file receiver a.k.a.
            // the destination handler
            FileDirectiveType::MetadataPdu
            | FileDirectiveType::EofPdu
            | FileDirectiveType::PromptPdu => PacketTarget::DestEntity,
            // Section a): Recipient depends of the type of PDU that is being acknowledged. We can simply
            // extract the PDU type from the raw stream. If it is an EOF PDU, this packet is passed to
            // the source handler, for a Finished PDU, it is passed to the destination handler.
            FileDirectiveType::AckPdu => {
                let acked_directive = FileDirectiveType::try_from(raw_packet[header_len + 1])
                    .map_err(|_| PduError::InvalidDirectiveType {
                        found: raw_packet[header_len],
                        expected: None,
                    })?;
                if acked_directive == FileDirectiveType::EofPdu {
                    PacketTarget::SourceEntity
                } else if acked_directive == FileDirectiveType::FinishedPdu {
                    PacketTarget::DestEntity
                } else {
                    // TODO: Maybe a better error? This might be confusing..
                    return Err(PduError::InvalidDirectiveType {
                        found: raw_packet[header_len + 1],
                        expected: None,
                    });
                }
            }
        };
        Ok(Self {
            pdu_type: pdu_header.pdu_type(),
            pdu_directive: Some(directive),
            target: packet_target,
            raw_packet,
        })
    }

    pub fn pdu_type(&self) -> PduType {
        self.pdu_type
    }

    pub fn pdu_directive(&self) -> Option<FileDirectiveType> {
        self.pdu_directive
    }

    pub fn target(&self) -> PacketTarget {
        self.target
    }

    pub fn raw_packet(&self) -> &[u8] {
        self.raw_packet
    }
}

#[cfg(test)]
mod tests {
    use spacepackets::cfdp::{
        lv::Lv,
        pdu::{
            eof::EofPdu,
            file_data::FileDataPdu,
            metadata::{MetadataGenericParams, MetadataPduCreator},
            CommonPduConfig, FileDirectiveType, PduHeader, WritablePduPacket,
        },
        PduType,
    };

    use crate::cfdp::PacketTarget;

    use super::PacketInfo;

    fn generic_pdu_header() -> PduHeader {
        let pdu_conf = CommonPduConfig::default();
        PduHeader::new_no_file_data(pdu_conf, 0)
    }

    #[test]
    fn test_metadata_pdu_info() {
        let mut buf: [u8; 128] = [0; 128];
        let pdu_header = generic_pdu_header();
        let metadata_params = MetadataGenericParams::default();
        let src_file_name = "hello.txt";
        let dest_file_name = "hello-dest.txt";
        let src_lv = Lv::new_from_str(src_file_name).unwrap();
        let dest_lv = Lv::new_from_str(dest_file_name).unwrap();
        let metadata_pdu =
            MetadataPduCreator::new_no_opts(pdu_header, metadata_params, src_lv, dest_lv);
        metadata_pdu
            .write_to_bytes(&mut buf)
            .expect("writing metadata PDU failed");

        let packet_info = PacketInfo::new(&buf).expect("creating packet info failed");
        assert_eq!(packet_info.pdu_type(), PduType::FileDirective);
        assert!(packet_info.pdu_directive().is_some());
        assert_eq!(
            packet_info.pdu_directive().unwrap(),
            FileDirectiveType::MetadataPdu
        );
        assert_eq!(packet_info.target(), PacketTarget::DestEntity);
    }

    #[test]
    fn test_filedata_pdu_info() {
        let mut buf: [u8; 128] = [0; 128];
        let pdu_header = generic_pdu_header();
        let file_data_pdu = FileDataPdu::new_no_seg_metadata(pdu_header, 0, &[]);
        file_data_pdu
            .write_to_bytes(&mut buf)
            .expect("writing file data PDU failed");
        let packet_info = PacketInfo::new(&buf).expect("creating packet info failed");
        assert_eq!(packet_info.pdu_type(), PduType::FileData);
        assert!(packet_info.pdu_directive().is_none());
        assert_eq!(packet_info.target(), PacketTarget::DestEntity);
    }

    #[test]
    fn test_eof_pdu_info() {
        let mut buf: [u8; 128] = [0; 128];
        let pdu_header = generic_pdu_header();
        let eof_pdu = EofPdu::new_no_error(pdu_header, 0, 0);
        eof_pdu
            .write_to_bytes(&mut buf)
            .expect("writing file data PDU failed");
        let packet_info = PacketInfo::new(&buf).expect("creating packet info failed");
        assert_eq!(packet_info.pdu_type(), PduType::FileDirective);
        assert!(packet_info.pdu_directive().is_some());
        assert_eq!(
            packet_info.pdu_directive().unwrap(),
            FileDirectiveType::EofPdu
        );
    }
}
