use crate::tmtc::{ReceivesCcsds, ReceivesTc};
use spacepackets::{CcsdsPacket, SpHeader};

pub trait ApidHandler {
    fn get_apid_handler(&self, apid: u16) -> Box<dyn ReceivesCcsds>;
}

struct CcsdsDistributor {
    apid_handlers: Box<dyn ApidHandler>,
}

impl ReceivesTc for CcsdsDistributor {
    fn pass_tc(&mut self, tm_raw: &[u8]) {
        // TODO: Better error handling
        let sp_header = SpHeader::from_raw_slice(tm_raw).unwrap();
        let mut handler = self.apid_handlers.get_apid_handler(sp_header.apid());
        handler.pass_ccsds(&sp_header, tm_raw).unwrap();
    }
}
