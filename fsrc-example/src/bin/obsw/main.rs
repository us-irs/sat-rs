mod pus;
mod tmtc;

use crate::tmtc::{core_tmtc_task, TmStore};
use fsrc_core::hal::host::udp_server::UdpTcServer;
use fsrc_core::pool::{LocalPool, PoolCfg, StoreAddr};
use fsrc_core::tmtc::CcsdsError;
use fsrc_example::{OBSW_SERVER_ADDR, SERVER_PORT};
use std::net::{IpAddr, SocketAddr};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

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
        core_tmtc_task(tm_creator_tx.clone(), tm_server_rx, tm_store.clone(), addr);
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
