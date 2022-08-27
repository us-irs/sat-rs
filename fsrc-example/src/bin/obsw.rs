use fsrc_core::hal::host::udp_server::{ReceiveResult, UdpTcServer};
use fsrc_core::pool::{LocalPool, PoolCfg, StoreAddr};
use fsrc_core::tmtc::{
    CcsdsDistributor, CcsdsError, CcsdsPacketHandler, PusDistributor, PusServiceProvider,
    ReceivesCcsdsTc,
};
use fsrc_example::{OBSW_SERVER_ADDR, SERVER_PORT};
use spacepackets::tc::PusTc;
use spacepackets::time::{CdsShortTimeProvider, TimeWriter};
use spacepackets::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::{CcsdsPacket, SpHeader};
use std::net::{IpAddr, SocketAddr};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

const PUS_APID: u16 = 0x02;

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

unsafe impl Send for CcsdsReceiver {}

struct PusReceiver {
    tm_apid: u16,
    tm_tx: mpsc::Sender<StoreAddr>,
    tm_store: Arc<Mutex<TmStore>>,
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
            let mut reply_header = SpHeader::tm(self.tm_apid, 0, 0).unwrap();
            let time_stamp = CdsShortTimeProvider::from_now().unwrap();
            let mut timestamp_buf = [0; 10];
            time_stamp.write_to_bytes(&mut timestamp_buf).unwrap();
            let tc_header = PusTmSecondaryHeader::new_simple(17, 2, &timestamp_buf);
            let ping_reply = PusTm::new(&mut reply_header, tc_header, None, true);
            let addr = self
                .tm_store
                .lock()
                .expect("Locking TM store failed")
                .add_pus_tm(&ping_reply);
            self.tm_tx
                .send(addr)
                .expect("Sending TM to TM funnel failed");
        }
        Ok(())
    }
}

struct TmStore {
    pool: LocalPool,
}

impl TmStore {
    fn add_pus_tm(&mut self, pus_tm: &PusTm) -> StoreAddr {
        let (addr, mut buf) = self
            .pool
            .free_element(pus_tm.len_packed())
            .expect("Store error");
        pus_tm
            .write_to(&mut buf)
            .expect("Writing PUS TM to store failed");
        addr
    }
}

struct TmFunnel {
    tm_funnel_rx: mpsc::Receiver<StoreAddr>,
    tm_server_tx: mpsc::Sender<StoreAddr>,
}

struct UdpTmtcServer {
    udp_tc_server: UdpTcServer<CcsdsError<()>>,
    tm_rx: mpsc::Receiver<StoreAddr>,
    tm_store: Arc<Mutex<TmStore>>,
}

unsafe impl Send for UdpTmtcServer {}

fn main() {
    let pool_cfg = PoolCfg::new(vec![(8, 32), (4, 64), (2, 128)]);
    let tm_pool = LocalPool::new(pool_cfg);
    let tm_store = Arc::new(Mutex::new(TmStore { pool: tm_pool }));
    let addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let (tm_creator_tx, tm_funnel_rx) = mpsc::channel();
    let (tm_server_tx, tm_server_rx) = mpsc::channel();

    let jh0 = thread::spawn(move || {
        let pus_receiver = PusReceiver {
            tm_apid: PUS_APID,
            tm_tx: tm_creator_tx.clone(),
            tm_store: tm_store.clone(),
        };
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
            tm_store: tm_store.clone(),
        };
        loop {
            let res = udp_tmtc_server.udp_tc_server.try_recv_tc();
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
                    ReceiveResult::OtherIoError(e) => {
                        println!("IO error {e}");
                    }
                    ReceiveResult::WouldBlock => {
                        // TODO: Send TM Here
                    }
                },
            }
        }
    });
    let jh1 = thread::spawn(move || {
        let tm_funnel = TmFunnel {
            tm_server_tx,
            tm_funnel_rx,
        };
        loop {
            let res = tm_funnel.tm_funnel_rx.recv();
            if res.is_ok() {
                let addr = res.unwrap();
                tm_funnel
                    .tm_server_tx
                    .send(addr)
                    .expect("Sending TM to server failed");
            }
        }
    });
    jh0.join().expect("Joining UDP TMTC server thread failed");
    jh1.join().expect("Joining TM Funnel thread failed");
}
