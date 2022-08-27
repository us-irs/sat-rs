use fsrc_core::hal::host::udp_server::{ReceiveResult, UdpTcServer};
use fsrc_core::tmtc::{
    CcsdsDistributor, CcsdsError, CcsdsPacketHandler, PusDistributor, PusServiceProvider,
    ReceivesCcsdsTc,
};
use fsrc_example::{OBSW_SERVER_ADDR, SERVER_PORT};
use spacepackets::tc::PusTc;
use spacepackets::{CcsdsPacket, SpHeader};
use std::net::{IpAddr, SocketAddr};

const PUS_APID: u16 = 0x02;

struct PusReceiver {}

struct CcsdsReceiver {
    pus_handler: PusDistributor<()>,
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

impl PusServiceProvider for PusReceiver {
    type Error = ();

    fn handle_pus_tc_packet(
        &mut self,
        service: u8,
        _header: &SpHeader,
        pus_tc: &PusTc,
    ) -> Result<(), Self::Error> {
        if service == 17 {
            println!("Received PUS ping command");
            let raw_data = pus_tc.raw().expect("Could not retrieve raw data");
            println!("Raw data: 0x{raw_data:x?}");
        }
        Ok(())
    }
}

fn main() {
    let addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let pus_receiver = PusReceiver {};
    let pus_distributor = PusDistributor::new(Box::new(pus_receiver));
    let ccsds_receiver = CcsdsReceiver {
        pus_handler: pus_distributor,
    };
    let ccsds_distributor = CcsdsDistributor::new(Box::new(ccsds_receiver));
    let mut udp_tmtc_server = UdpTcServer::new(addr, 2048, Box::new(ccsds_distributor))
        .expect("Creating UDP TMTC server failed");
    loop {
        let res = udp_tmtc_server.recv_tc();
        match res {
            Ok(_) => (),
            Err(e) => match e {
                ReceiveResult::ReceiverError(e) => match e {
                    CcsdsError::PacketError(e) => {
                        println!("Got packet error: {e:?}");
                    }
                    CcsdsError::CustomError(_) => {
                        println!("Unknown receiver error")
                    }
                },
                ReceiveResult::IoError(e) => {
                    println!("IO error {e}");
                }
            },
        }
    }
}
