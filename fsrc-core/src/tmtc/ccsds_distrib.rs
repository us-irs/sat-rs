use crate::error::FsrcErrorHandler;
use crate::tmtc::{
    ReceivesCcsds, ReceivesTc, FROM_BYTES_SLICE_TOO_SMALL_ERROR, FROM_BYTES_ZEROCOPY_ERROR,
};
use spacepackets::{CcsdsPacket, PacketError, SpHeader};

pub trait HandlesPacketForApid {
    fn get_apid_handler(&mut self, apid: u16) -> Option<&mut Box<dyn ReceivesCcsds>>;
    fn handle_unknown_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]);
}

pub struct CcsdsDistributor {
    apid_handlers: Box<dyn HandlesPacketForApid>,
    error_handler: Box<dyn FsrcErrorHandler>,
}

impl CcsdsDistributor {
    pub fn new(
        apid_handlers: Box<dyn HandlesPacketForApid>,
        error_handler: Box<dyn FsrcErrorHandler>,
    ) -> Self {
        CcsdsDistributor {
            apid_handlers,
            error_handler,
        }
    }
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
        let apid = sp_header.apid();
        match self.apid_handlers.get_apid_handler(apid) {
            None => {
                self.apid_handlers.handle_unknown_apid(&sp_header, tm_raw);
            }
            Some(handler) => {
                handler.pass_ccsds(&sp_header, tm_raw).ok();
            }
        }
    }
}
