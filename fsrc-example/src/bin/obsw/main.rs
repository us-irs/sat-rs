mod ccsds;
mod pus;
mod tmtc;

use crate::tmtc::{core_tmtc_task, TmStore, PUS_APID};
use fsrc_core::hal::host::udp_server::UdpTcServer;
use fsrc_core::pool::{LocalPool, PoolCfg, SharedPool, StoreAddr};
use fsrc_core::pus::verification::{
    MpscVerifSender, VerificationReporterCfg, VerificationReporterWithSender,
};
use fsrc_core::tmtc::CcsdsError;
use fsrc_example::{OBSW_SERVER_ADDR, SERVER_PORT};
use std::net::{IpAddr, SocketAddr};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;

struct TmFunnel {
    tm_funnel_rx: mpsc::Receiver<StoreAddr>,
    tm_server_tx: mpsc::Sender<StoreAddr>,
}

struct UdpTmtcServer {
    udp_tc_server: UdpTcServer<CcsdsError<()>>,
    tm_rx: mpsc::Receiver<StoreAddr>,
    tm_store: SharedPool,
}

unsafe impl Send for UdpTmtcServer {}

fn main() {
    println!("Running OBSW example");
    let pool_cfg = PoolCfg::new(vec![(8, 32), (4, 64), (2, 128)]);
    let tm_pool = LocalPool::new(pool_cfg);
    let tm_store: SharedPool = Arc::new(RwLock::new(Box::new(tm_pool)));
    let tm_store_helper = TmStore {
        pool: tm_store.clone(),
    };
    let addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), SERVER_PORT);
    let (tm_funnel_tx, tm_funnel_rx) = mpsc::channel();
    let (tm_server_tx, tm_server_rx) = mpsc::channel();
    let sender = MpscVerifSender::new(tm_store.clone(), tm_funnel_tx.clone());
    let verif_cfg = VerificationReporterCfg::new(PUS_APID, 1, 2, 8);
    let reporter_with_sender_0 = Arc::new(Mutex::new(VerificationReporterWithSender::new(
        verif_cfg,
        Box::new(sender),
    )));
    let jh0 = thread::spawn(move || {
        core_tmtc_task(
            tm_funnel_tx.clone(),
            tm_server_rx,
            tm_store_helper.clone(),
            addr,
            reporter_with_sender_0,
        );
    });

    let jh1 = thread::spawn(move || {
        let tm_funnel = TmFunnel {
            tm_server_tx,
            tm_funnel_rx,
        };
        loop {
            if let Ok(addr) = tm_funnel.tm_funnel_rx.recv() {
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
