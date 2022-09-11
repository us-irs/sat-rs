use fsrc_core::hal::host::udp_server::{ReceiveResult, UdpTcServer};
use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::ccsds::CcsdsReceiver;
use crate::pus::PusReceiver;
use crate::UdpTmtcServer;
use fsrc_core::pool::{SharedPool, StoreAddr};
use fsrc_core::pus::verification::SharedStdVerifReporterWithSender;
use fsrc_core::tmtc::{CcsdsDistributor, CcsdsError, PusDistributor};
use spacepackets::tm::PusTm;

pub const PUS_APID: u16 = 0x02;

#[derive(Clone)]
pub struct TmStore {
    pub pool: SharedPool,
}

impl TmStore {
    pub fn add_pus_tm(&mut self, pus_tm: &PusTm) -> StoreAddr {
        let mut pg = self.pool.write().expect("Error locking TM store");
        let (addr, buf) = pg.free_element(pus_tm.len_packed()).expect("Store error");
        pus_tm
            .write_to_bytes(buf)
            .expect("Writing PUS TM to store failed");
        addr
    }
}

pub fn core_tmtc_task(
    tm_creator_tx: mpsc::Sender<StoreAddr>,
    tm_server_rx: mpsc::Receiver<StoreAddr>,
    tm_store_helper: TmStore,
    addr: SocketAddr,
    verif_reporter: SharedStdVerifReporterWithSender,
) {
    let pus_receiver = PusReceiver::new(
        PUS_APID,
        tm_creator_tx,
        tm_store_helper.clone(),
        verif_reporter,
    );
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
        tm_store: tm_store_helper.pool.clone(),
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
            ReceiveResult::IoError(e) => {
                println!("IO error {e}");
                false
            }
            ReceiveResult::NothingReceived => false,
        },
    }
}

fn core_tm_handling(udp_tmtc_server: &mut UdpTmtcServer, recv_addr: &SocketAddr) {
    while let Ok(addr) = udp_tmtc_server.tm_rx.try_recv() {
        let mut store_lock = udp_tmtc_server
            .tm_store
            .write()
            .expect("Locking TM store failed");
        let pg = store_lock.read_with_guard(addr);
        let buf = pg.read().expect("Error reading TM pool data");
        println!("Sending TM");
        udp_tmtc_server
            .udp_tc_server
            .socket
            .send_to(buf, recv_addr)
            .expect("Sending TM failed");
    }
}
