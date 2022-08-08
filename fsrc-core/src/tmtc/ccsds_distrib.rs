use crate::error::FsrcErrorHandler;
use crate::tmtc::{
    ReceivesCcsds, ReceivesTc, FROM_BYTES_SLICE_TOO_SMALL_ERROR, FROM_BYTES_ZEROCOPY_ERROR,
};
use spacepackets::{CcsdsPacket, PacketError, SpHeader};

pub trait ApidHandler {
    fn get_apid_handler(&self, apid: u16) -> Box<dyn ReceivesCcsds>;
}

struct CcsdsDistributor {
    error_handler: Box<dyn FsrcErrorHandler>,
    apid_handlers: Box<dyn ApidHandler>,
}

impl ReceivesTc for CcsdsDistributor {
    fn pass_tc(&mut self, tm_raw: &[u8]) {
        let sp_header = match SpHeader::from_raw_slice(tm_raw) {
            Ok(header) => header,
            Err(e) => {
                match e {
                    PacketError::FromBytesSliceTooSmall(missmatch) => {
                        self.error_handler.error_with_two_params(
                            FROM_BYTES_SLICE_TOO_SMALL_ERROR,
                            missmatch.found as u32,
                            missmatch.expected as u32,
                        );
                    }
                    PacketError::FromBytesZeroCopyError => {
                        self.error_handler.error(FROM_BYTES_ZEROCOPY_ERROR);
                    }
                    _ => {
                        // TODO: Unexpected error
                    }
                }
                return;
            }
        };
        let mut handler = self.apid_handlers.get_apid_handler(sp_header.apid());
        handler.pass_ccsds(&sp_header, tm_raw).unwrap();
    }
}
