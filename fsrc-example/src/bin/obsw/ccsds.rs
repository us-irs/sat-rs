use crate::tmtc::PUS_APID;
use fsrc_core::tmtc::{CcsdsPacketHandler, PusDistributor, ReceivesCcsdsTc};
use spacepackets::{CcsdsPacket, SpHeader};

pub struct CcsdsReceiver {
    pub pus_handler: PusDistributor<()>,
}

impl CcsdsPacketHandler for CcsdsReceiver {
    type Error = ();

    fn valid_apids(&self) -> &'static [u16] {
        &[PUS_APID]
    }

    fn handle_known_apid(
        &mut self,
        sp_header: &SpHeader,
        tc_raw: &[u8],
    ) -> Result<(), Self::Error> {
        if sp_header.apid() == PUS_APID {
            self.pus_handler
                .pass_ccsds(sp_header, tc_raw)
                .expect("Handling PUS packet failed");
        }
        Ok(())
    }

    fn handle_unknown_apid(
        &mut self,
        _sp_header: &SpHeader,
        _tc_raw: &[u8],
    ) -> Result<(), Self::Error> {
        println!("Unknown APID detected");
        Ok(())
    }
}
