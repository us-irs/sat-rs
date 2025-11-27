extern crate alloc;

use satrs::spacepackets::{
    ccsds_packet_len_for_user_data_len_with_checksum, time::cds::CdsTime, CcsdsPacketCreationError,
    CcsdsPacketCreatorWithReservedData, CcsdsPacketIdAndPsc, SpacePacketHeader,
};

pub mod config;
pub mod ids;

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    num_enum::TryFromPrimitive,
    num_enum::IntoPrimitive,
)]
#[repr(u64)]
pub enum ComponentId {
    Pcdu,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MessageType {
    Ping,
    Mode,
    Hk,
    Action,
    Verification,
}

/// Unserialized owned TM packet which can be cloned and sent around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcsdsTmPacketOwned {
    pub sp_header: SpacePacketHeader,
    pub tm_header: TmHeader,
    pub payload: alloc::vec::Vec<u8>,
}

/// Simple type modelling packet stored in the heap. This structure is intended to
/// be used when sending a packet via a message queue, so it also contains the sender ID.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PacketAsVec {
    pub sender_id: ComponentId,
    pub packet: Vec<u8>,
}

impl PacketAsVec {
    pub fn new(sender_id: ComponentId, packet: Vec<u8>) -> Self {
        Self { sender_id, packet }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CcsdsCreationError {
    #[error("CCSDS packet creation error: {0}")]
    CcsdsPacketCreation(#[from] CcsdsPacketCreationError),
    #[error("postcard error: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("timestamp generation error")]
    Time,
}

impl CcsdsTmPacketOwned {
    pub fn write_to_bytes(&self, buf: &mut [u8]) -> Result<usize, CcsdsCreationError> {
        let response_len =
            postcard::experimental::serialized_size(&self.tm_header)? + self.payload.len();
        let mut ccsds_tm = CcsdsPacketCreatorWithReservedData::new_tm_with_checksum(
            self.sp_header,
            response_len,
            buf,
        )?;
        let user_data = ccsds_tm.packet_data_mut();
        let ser_len = postcard::to_slice(&self.tm_header, user_data)?.len();
        user_data[ser_len..ser_len + self.payload.len()].copy_from_slice(&self.payload);
        let ccsds_packet_len = ccsds_tm.finish();
        Ok(ccsds_packet_len)
    }

    pub fn len_written(&self) -> usize {
        ccsds_packet_len_for_user_data_len_with_checksum(
            postcard::experimental::serialized_size(&self.tm_header).unwrap() as usize
                + postcard::experimental::serialized_size(&self.payload).unwrap() as usize,
        )
        .unwrap()
    }

    pub fn to_vec(&self) -> alloc::vec::Vec<u8> {
        let mut buf = alloc::vec![0u8; self.len_written()];
        let len = self.write_to_bytes(&mut buf).unwrap();
        buf.truncate(len);
        buf
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct TmHeader {
    pub sender_id: ComponentId,
    pub target_id: ComponentId,
    pub message_type: MessageType,
    /// Telemetry can either be sent unsolicited, or as a response to telecommands.
    pub tc_id: Option<CcsdsPacketIdAndPsc>,
    /// Raw CDS short timestamp.
    pub timestamp: Option<[u8; 7]>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct TcHeader {
    pub target_id: ComponentId,
    pub request_type: MessageType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcsdsTcPacketOwned {
    pub sp_header: SpacePacketHeader,
    pub tc_header: TcHeader,
    pub payload: alloc::vec::Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum DeviceMode {
    Off = 0,
    On = 1,
    Normal = 2,
}

pub struct TimestampHelper {
    stamper: CdsTime,
    time_stamp: [u8; 7],
}

impl TimestampHelper {
    pub fn stamp(&self) -> &[u8] {
        &self.time_stamp
    }

    pub fn update_from_now(&mut self) {
        self.stamper
            .update_from_now()
            .expect("Updating timestamp failed");
        self.stamper
            .write_to_bytes(&mut self.time_stamp)
            .expect("Writing timestamp failed");
    }
}

impl Default for TimestampHelper {
    fn default() -> Self {
        Self {
            stamper: CdsTime::now_with_u16_days().expect("creating time stamper failed"),
            time_stamp: Default::default(),
        }
    }
}
