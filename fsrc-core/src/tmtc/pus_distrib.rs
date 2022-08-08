use crate::error::FsrcErrorHandler;
use crate::tmtc::{ReceivesCcsds, ReceivesPus, ReceivesTc};
use spacepackets::ecss::PusPacket;
use spacepackets::tc::PusTc;
use spacepackets::{CcsdsPacket, PacketError, SpHeader};

pub trait PusServiceProvider {
    fn get_apid(&self, service: u8) -> u16;
    fn get_service_handler(&self, service: u8, subservice: u8) -> Box<dyn ReceivesPus>;
}

pub struct PusDistributor {
    error_handler: Box<dyn FsrcErrorHandler>,
    service_provider: Box<dyn PusServiceProvider>,
}

impl ReceivesTc for PusDistributor {
    fn pass_tc(&mut self, tm_raw: &[u8]) {
        // Convert to ccsds and call pass_ccsds
        let sp_header = SpHeader::from_raw_slice(tm_raw).unwrap();
        self.pass_ccsds(&sp_header, tm_raw).unwrap();
    }
}

impl ReceivesCcsds for PusDistributor {
    fn pass_ccsds(&mut self, _header: &SpHeader, tm_raw: &[u8]) -> Result<(), PacketError> {
        // TODO: Better error handling
        let (tc, _) = PusTc::new_from_raw_slice(tm_raw).unwrap();

        let mut srv_provider = self
            .service_provider
            .get_service_handler(tc.service(), tc.subservice());
        let apid = self.service_provider.get_apid(tc.service());
        if apid != tc.apid() {
            // TODO: Dedicated error
            return Ok(());
        }
        srv_provider.pass_pus(&tc).unwrap();
        Ok(())
    }
}
