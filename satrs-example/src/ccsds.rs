use satrs::pus::ReceivesEcssPusTc;
use satrs::spacepackets::{CcsdsPacket, SpHeader};
use satrs::tmtc::{CcsdsPacketHandler, ReceivesCcsdsTc};
use satrs_example::config::PUS_APID;

#[derive(Clone)]
pub struct CcsdsReceiver<
    TcSource: ReceivesCcsdsTc<Error = E> + ReceivesEcssPusTc<Error = E> + Clone,
    E,
> {
    pub tc_source: TcSource,
}

impl<
        TcSource: ReceivesCcsdsTc<Error = E> + ReceivesEcssPusTc<Error = E> + Clone + 'static,
        E: 'static,
    > CcsdsPacketHandler for CcsdsReceiver<TcSource, E>
{
    type Error = E;

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
