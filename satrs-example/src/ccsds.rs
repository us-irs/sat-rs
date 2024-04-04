use satrs::pus::ReceivesEcssPusTc;
use satrs::spacepackets::{CcsdsPacket, SpHeader};
use satrs::tmtc::{CcsdsPacketHandler, ReceivesCcsdsTc};
use satrs::ValidatorU16Id;
use satrs_example::config::components::Apid;
use satrs_example::config::APID_VALIDATOR;

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
    > ValidatorU16Id for CcsdsReceiver<TcSource, E>
{
    fn validate(&self, apid: u16) -> bool {
        APID_VALIDATOR.contains(&apid)
    }
}

impl<
        TcSource: ReceivesCcsdsTc<Error = E> + ReceivesEcssPusTc<Error = E> + Clone + 'static,
        E: 'static,
    > CcsdsPacketHandler for CcsdsReceiver<TcSource, E>
{
    type Error = E;

    fn handle_packet_with_valid_apid(
        &mut self,
        sp_header: &SpHeader,
        tc_raw: &[u8],
    ) -> Result<(), Self::Error> {
        if sp_header.apid() == Apid::Cfdp as u16 {
        } else {
            return self.tc_source.pass_ccsds(sp_header, tc_raw);
        }
        Ok(())
    }

    fn handle_packet_with_unknown_apid(
        &mut self,
        sp_header: &SpHeader,
        _tc_raw: &[u8],
    ) -> Result<(), Self::Error> {
        log::warn!("unknown APID 0x{:x?} detected", sp_header.apid());
        Ok(())
    }
}
