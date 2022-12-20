use satrs_core::events::EventU32;
use satrs_core::hal::host::udp_server::{ReceiveResult, UdpTcServer};
use satrs_core::params::Params;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use crate::ccsds::CcsdsReceiver;
use crate::pus::PusReceiver;
use crate::requests::Request;
use crate::UdpTmtcServer;
use satrs_core::pool::{SharedPool, StoreAddr};
use satrs_core::pus::event_man::EventRequestWithToken;
use satrs_core::pus::verification::StdVerifReporterWithSender;
use satrs_core::tmtc::{CcsdsDistributor, CcsdsError, PusDistributor};
use spacepackets::tm::PusTm;

pub const PUS_APID: u16 = 0x02;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum RequestTargetId {
    AcsSubsystem = 1,
}

pub struct CoreTmtcArgs {
    pub tm_store: TmStore,
    pub tm_sender: Sender<StoreAddr>,
    pub event_sender: Sender<(EventU32, Option<Params>)>,
    pub event_request_tx: Sender<EventRequestWithToken>,
    pub request_map: HashMap<u32, Sender<Request>>,
}

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
    args: CoreTmtcArgs,
    tm_server_rx: mpsc::Receiver<StoreAddr>,
    addr: SocketAddr,
    verif_reporter: StdVerifReporterWithSender,
) {
    let pus_receiver = PusReceiver::new(
        PUS_APID,
        args.tm_sender,
        args.tm_store.clone(),
        verif_reporter,
        args.event_request_tx,
        args.request_map,
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
        tm_store: args.tm_store.pool.clone(),
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
        if buf.len() > 9 {
            let service = buf[7];
            let subservice = buf[8];
            println!("Sending PUS TM[{},{}]", service, subservice)
        } else {
            println!("Sending PUS TM");
        }
        udp_tmtc_server
            .udp_tc_server
            .socket
            .send_to(buf, recv_addr)
            .expect("Sending TM failed");
    }
}
