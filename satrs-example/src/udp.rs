use std::{
    net::{SocketAddr, UdpSocket},
    sync::mpsc::Receiver,
};

use log::{info, warn};
use satrs_core::{
    hal::std::udp_server::{ReceiveResult, UdpTcServer},
    pool::{PoolProviderWithGuards, SharedStaticMemoryPool, StoreAddr},
    tmtc::CcsdsError,
};

pub trait UdpTmHandler {
    fn send_tm_to_udp_client(&mut self, socket: &UdpSocket, recv_addr: &SocketAddr);
}

pub struct StaticUdpTmHandler {
    pub tm_rx: Receiver<StoreAddr>,
    pub tm_store: SharedStaticMemoryPool,
}

impl UdpTmHandler for StaticUdpTmHandler {
    fn send_tm_to_udp_client(&mut self, socket: &UdpSocket, &recv_addr: &SocketAddr) {
        while let Ok(addr) = self.tm_rx.try_recv() {
            let store_lock = self.tm_store.write();
            if store_lock.is_err() {
                warn!("Locking TM store failed");
                continue;
            }
            let mut store_lock = store_lock.unwrap();
            let pg = store_lock.read_with_guard(addr);
            let read_res = pg.read_as_vec();
            if read_res.is_err() {
                warn!("Error reading TM pool data");
                continue;
            }
            let buf = read_res.unwrap();
            let result = socket.send_to(&buf, recv_addr);
            if let Err(e) = result {
                warn!("Sending TM with UDP socket failed: {e}")
            }
        }
    }
}

pub struct DynamicUdpTmHandler {
    pub tm_rx: Receiver<Vec<u8>>,
}

impl UdpTmHandler for DynamicUdpTmHandler {
    fn send_tm_to_udp_client(&mut self, socket: &UdpSocket, recv_addr: &SocketAddr) {
        while let Ok(tm) = self.tm_rx.try_recv() {
            if tm.len() > 9 {
                let service = tm[7];
                let subservice = tm[8];
                info!("Sending PUS TM[{service},{subservice}]")
            } else {
                info!("Sending PUS TM");
            }
            let result = socket.send_to(&tm, recv_addr);
            if let Err(e) = result {
                warn!("Sending TM with UDP socket failed: {e}")
            }
        }
    }
}

pub struct UdpTmtcServer<TmHandler: UdpTmHandler, SendError> {
    pub udp_tc_server: UdpTcServer<CcsdsError<SendError>>,
    pub tm_handler: TmHandler,
}

impl<TmHandler: UdpTmHandler, SendError: core::fmt::Debug + 'static>
    UdpTmtcServer<TmHandler, SendError>
{
    pub fn periodic_operation(&mut self) {
        while self.poll_tc_server() {}
        if let Some(recv_addr) = self.udp_tc_server.last_sender() {
            self.tm_handler
                .send_tm_to_udp_client(&self.udp_tc_server.socket, &recv_addr);
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
                        warn!("mpsc custom error {e:?}");
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
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        net::IpAddr,
        sync::{Arc, Mutex},
    };

    use satrs_core::{
        spacepackets::{
            ecss::{tc::PusTcCreator, WritablePusPacket},
            SpHeader,
        },
        tmtc::ReceivesTcCore,
    };
    use satrs_example::config::{OBSW_SERVER_ADDR, PUS_APID};

    use super::*;

    #[derive(Default, Debug, Clone)]
    pub struct TestReceiver {
        tc_vec: Arc<Mutex<VecDeque<Vec<u8>>>>,
    }

    impl ReceivesTcCore for TestReceiver {
        type Error = CcsdsError<()>;
        fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
            self.tc_vec.lock().unwrap().push_back(tc_raw.to_vec());
            Ok(())
        }
    }

    #[derive(Default, Debug, Clone)]
    pub struct TestTmHandler {
        addrs_to_send_to: Arc<Mutex<VecDeque<SocketAddr>>>,
    }

    impl UdpTmHandler for TestTmHandler {
        fn send_tm_to_udp_client(&mut self, _socket: &UdpSocket, recv_addr: &SocketAddr) {
            self.addrs_to_send_to.lock().unwrap().push_back(*recv_addr);
        }
    }

    #[test]
    fn test_basic() {
        let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), 0);
        let test_receiver = TestReceiver::default();
        let tc_queue = test_receiver.tc_vec.clone();
        let udp_tc_server = UdpTcServer::new(sock_addr, 2048, Box::new(test_receiver)).unwrap();
        let tm_handler = TestTmHandler::default();
        let tm_handler_calls = tm_handler.addrs_to_send_to.clone();
        let mut udp_dyn_server = UdpTmtcServer {
            udp_tc_server,
            tm_handler,
        };
        udp_dyn_server.periodic_operation();
        assert!(tc_queue.lock().unwrap().is_empty());
        assert!(tm_handler_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn test_transactions() {
        let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), 0);
        let test_receiver = TestReceiver::default();
        let tc_queue = test_receiver.tc_vec.clone();
        let udp_tc_server = UdpTcServer::new(sock_addr, 2048, Box::new(test_receiver)).unwrap();
        let server_addr = udp_tc_server.socket.local_addr().unwrap();
        let tm_handler = TestTmHandler::default();
        let tm_handler_calls = tm_handler.addrs_to_send_to.clone();
        let mut udp_dyn_server = UdpTmtcServer {
            udp_tc_server,
            tm_handler,
        };
        let mut sph = SpHeader::tc_unseg(PUS_APID, 0, 0).unwrap();
        let ping_tc = PusTcCreator::new_simple(&mut sph, 17, 1, None, true)
            .to_vec()
            .unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").expect("Connecting to UDP server failed");
        let client_addr = client.local_addr().unwrap();
        client.connect(server_addr).unwrap();
        client.send(&ping_tc).unwrap();
        udp_dyn_server.periodic_operation();
        {
            let mut tc_queue = tc_queue.lock().unwrap();
            assert!(!tc_queue.is_empty());
            let received_tc = tc_queue.pop_front().unwrap();
            assert_eq!(received_tc, ping_tc);
        }

        {
            let mut tm_handler_calls = tm_handler_calls.lock().unwrap();
            assert!(!tm_handler_calls.is_empty());
            assert_eq!(tm_handler_calls.len(), 1);
            let received_addr = tm_handler_calls.pop_front().unwrap();
            assert_eq!(received_addr, client_addr);
        }
        udp_dyn_server.periodic_operation();
        assert!(tc_queue.lock().unwrap().is_empty());
        // Still tries to send to the same client.
        {
            let mut tm_handler_calls = tm_handler_calls.lock().unwrap();
            assert!(!tm_handler_calls.is_empty());
            assert_eq!(tm_handler_calls.len(), 1);
            let received_addr = tm_handler_calls.pop_front().unwrap();
            assert_eq!(received_addr, client_addr);
        }
    }
}
