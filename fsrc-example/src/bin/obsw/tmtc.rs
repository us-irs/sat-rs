use fsrc_core::hal::host::udp_server::{ReceiveResult, UdpTcServer};
use std::net::SocketAddr;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::UdpTmtcServer;
use fsrc_core::pool::{LocalPool, StoreAddr};
use fsrc_core::tmtc::tm_helper::PusTmWithCdsShortHelper;
use fsrc_core::tmtc::{
    CcsdsDistributor, CcsdsError, CcsdsPacketHandler, PusDistributor, PusServiceProvider,
    ReceivesCcsdsTc,
};
use spacepackets::tc::PusTc;
use spacepackets::tm::PusTm;
use spacepackets::{CcsdsPacket, SpHeader};

pub const PUS_APID: u16 = 0x02;

pub struct TmStore {
    pub pool: LocalPool,
}

impl TmStore {
    fn add_pus_tm(&mut self, pus_tm: &PusTm) -> StoreAddr {
        let (addr, buf) = self
            .pool
            .free_element(pus_tm.len_packed())
            .expect("Store error");
        pus_tm
            .write_to(buf)
            .expect("Writing PUS TM to store failed");
        addr
    }
}

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

unsafe impl Send for CcsdsReceiver {}

pub struct PusReceiver {
    pub tm_helper: PusTmWithCdsShortHelper,
    pub tm_tx: mpsc::Sender<StoreAddr>,
    pub tm_store: Arc<Mutex<TmStore>>,
}

impl PusReceiver {
    pub fn new(apid: u16, tm_tx: mpsc::Sender<StoreAddr>, tm_store: Arc<Mutex<TmStore>>) -> Self {
        Self {
            tm_helper: PusTmWithCdsShortHelper::new(apid),
            tm_tx,
            tm_store,
        }
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
            self.handle_test_service(pus_tc);
        }
        Ok(())
    }
}

impl PusReceiver {
    fn handle_test_service(&mut self, pus_tc: &PusTc) {
        println!("Received PUS ping command");
        let raw_data = pus_tc.raw().expect("Could not retrieve raw data");
        println!("Raw data: 0x{raw_data:x?}");
        println!("Sending ping reply");
        let ping_reply = self.tm_helper.create_pus_tm_timestamp_now(17, 2, None);
        let addr = self
            .tm_store
            .lock()
            .expect("Locking TM store failed")
            .add_pus_tm(&ping_reply);
        self.tm_tx
            .send(addr)
            .expect("Sending TM to TM funnel failed");
    }
}

pub fn core_tmtc_task(
    tm_creator_tx: mpsc::Sender<StoreAddr>,
    tm_server_rx: mpsc::Receiver<StoreAddr>,
    tm_store: Arc<Mutex<TmStore>>,
    addr: SocketAddr,
) {
    let pus_receiver = PusReceiver::new(PUS_APID, tm_creator_tx, tm_store.clone());
    let pus_distributor = PusDistributor::new(Box::new(pus_receiver));
    let ccsds_receiver = CcsdsReceiver {
        pus_handler: pus_distributor,
    };
    let ccsds_distributor = CcsdsDistributor::new(Box::new(ccsds_receiver));
    let udp_tc_server = UdpTcServer::new(addr, 2048, Box::new(ccsds_distributor))
        .expect("Creating UDP TMTC server failed");
    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_rx: tm_server_rx,
        tm_store,
    };
    loop {
        core_tmtc_loop(&mut udp_tmtc_server);
        thread::sleep(Duration::from_millis(400));
    }
}

fn core_tmtc_loop(udp_tmtc_server: &mut UdpTmtcServer) {
    while core_tc_handling(udp_tmtc_server) {}
    if let Some(recv_addr) = udp_tmtc_server.udp_tc_server.last_sender() {
        core_tm_handling(udp_tmtc_server, &recv_addr);
    }
}

fn core_tc_handling(udp_tmtc_server: &mut UdpTmtcServer) -> bool {
    match udp_tmtc_server.udp_tc_server.try_recv_tc() {
        Ok(_) => true,
        Err(e) => match e {
            ReceiveResult::ReceiverError(e) => match e {
                CcsdsError::PacketError(e) => {
                    println!("Got packet error: {e:?}");
                    true
                }
                CcsdsError::CustomError(_) => {
                    println!("Unknown receiver error");
                    true
                }
            },
            ReceiveResult::OtherIoError(e) => {
                println!("IO error {e}");
                false
            }
            ReceiveResult::WouldBlock => false,
        },
    }
}

fn core_tm_handling(udp_tmtc_server: &mut UdpTmtcServer, recv_addr: &SocketAddr) {
    while let Ok(addr) = udp_tmtc_server.tm_rx.try_recv() {
        let mut store_lock = udp_tmtc_server
            .tm_store
            .lock()
            .expect("Locking TM store failed");
        let pg = store_lock.pool.read_with_guard(addr);
        let buf = pg.read().expect("Error reading TM pool data");
        println!("Sending TM");
        udp_tmtc_server
            .udp_tc_server
            .socket
            .send_to(buf, recv_addr)
            .expect("Sending TM failed");
    }
}
