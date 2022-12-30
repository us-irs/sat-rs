use crate::tmtc::{MpscStoreAndSendError, PusTcSource, PUS_APID};
use satrs_core::tmtc::{CcsdsPacketHandler, ReceivesCcsdsTc};
use spacepackets::{CcsdsPacket, SpHeader};

pub struct CcsdsReceiver {
    pub tc_source: PusTcSource,
}

impl CcsdsPacketHandler for CcsdsReceiver {
    type Error = MpscStoreAndSendError;

    fn valid_apids(&self) -> &'static [u16] {
        &[PUS_APID]
    }

    fn handle_known_apid(
        &mut self,
        sp_header: &SpHeader,
        tc_raw: &[u8],
    ) -> Result<(), Self::Error> {
        if sp_header.apid() == PUS_APID {
            return self.tc_source.pass_ccsds(sp_header, tc_raw);
        }
        Ok(())
    }

    fn handle_unknown_apid(
        &mut self,
        sp_header: &SpHeader,
        _tc_raw: &[u8],
    ) -> Result<(), Self::Error> {
        println!("Unknown APID 0x{:x?} detected", sp_header.apid());
        Ok(())
    }
}
