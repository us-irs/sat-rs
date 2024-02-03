use std::{net::SocketAddr, sync::mpsc::Receiver};

use log::{info, warn};
use satrs_core::{
    hal::std::udp_server::{ReceiveResult, UdpTcServer},
    pool::{PoolProviderMemInPlaceWithGuards, SharedStaticMemoryPool, StoreAddr},
    tmtc::CcsdsError,
};

use crate::tmtc::MpscStoreAndSendError;

pub struct UdpTmtcServer {
    pub udp_tc_server: UdpTcServer<CcsdsError<MpscStoreAndSendError>>,
    pub tm_rx: Receiver<StoreAddr>,
    pub tm_store: SharedStaticMemoryPool,
}
impl UdpTmtcServer {
    pub fn periodic_operation(&mut self) {
        while self.poll_tc_server() {}
        if let Some(recv_addr) = self.udp_tc_server.last_sender() {
            self.send_tm_to_udp_client(&recv_addr);
        }
    }

    fn poll_tc_server(&mut self) -> bool {
        match self.udp_tc_server.try_recv_tc() {
            Ok(_) => true,
            Err(e) => match e {
                ReceiveResult::ReceiverError(e) => match e {
                    CcsdsError::ByteConversionError(e) => {
                        warn!("packet error: {e:?}");
                        true
                    }
                    CcsdsError::CustomError(e) => {
                        warn!("mpsc store and send error {e:?}");
                        true
                    }
                },
                ReceiveResult::IoError(e) => {
                    warn!("IO error {e}");
                    false
                }
                ReceiveResult::NothingReceived => false,
            },
        }
    }

    fn send_tm_to_udp_client(&mut self, recv_addr: &SocketAddr) {
        while let Ok(addr) = self.tm_rx.try_recv() {
            let store_lock = self.tm_store.write();
            if store_lock.is_err() {
                warn!("Locking TM store failed");
                continue;
            }
            let mut store_lock = store_lock.unwrap();
            let pg = store_lock.read_with_guard(addr);
            let read_res = pg.read();
            if read_res.is_err() {
                warn!("Error reading TM pool data");
                continue;
            }
            let buf = read_res.unwrap();
            if buf.len() > 9 {
                let service = buf[7];
                let subservice = buf[8];
                info!("Sending PUS TM[{service},{subservice}]")
            } else {
                info!("Sending PUS TM");
            }
            let result = self.udp_tc_server.socket.send_to(buf, recv_addr);
            if let Err(e) = result {
                warn!("Sending TM with UDP socket failed: {e}")
            }
        }
    }
}
