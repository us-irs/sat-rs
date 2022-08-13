use crate::error::FsrcErrorHandler;
use crate::tmtc::{ReceivesTc, FROM_BYTES_SLICE_TOO_SMALL_ERROR, FROM_BYTES_ZEROCOPY_ERROR};
use spacepackets::{CcsdsPacket, PacketError, SpHeader};

pub trait HandlesPacketForApid {
    fn valid_apids(&self) -> &'static [u16];
    fn handle_known_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]);
    fn handle_unknown_apid(&mut self, sp_header: &SpHeader, tc_raw: &[u8]);
}

pub struct CcsdsDistributor {
    apid_handler: Box<dyn HandlesPacketForApid>,
    error_handler: Box<dyn FsrcErrorHandler>,
}

impl<'a> CcsdsDistributor {
    pub fn new(
        apid_handler: Box<dyn HandlesPacketForApid>,
        error_handler: Box<dyn FsrcErrorHandler>,
    ) -> Self {
        CcsdsDistributor {
            apid_handler,
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
        let valid_apids = self.apid_handler.valid_apids();
        for &valid_apid in valid_apids {
            if valid_apid == apid {
                self.apid_handler.handle_known_apid(&sp_header, tm_raw);
            }
        }
        self.apid_handler.handle_unknown_apid(&sp_header, tm_raw);
    }
}
