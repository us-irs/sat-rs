use core::fmt::Debug;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;

use log::{info, warn};
use satrs::tmtc::{PacketAsVec, PacketInPool, PacketSenderRaw};
use satrs::{
    hal::std::udp_server::{ReceiveResult, UdpTcServer},
    pool::{PoolProviderWithGuards, SharedStaticMemoryPool},
};

use crate::pus::HandlingStatus;

pub trait UdpTmHandler {
    fn send_tm_to_udp_client(&mut self, socket: &UdpSocket, recv_addr: &SocketAddr);
}

pub struct StaticUdpTmHandler {
    pub tm_rx: mpsc::Receiver<PacketInPool>,
    pub tm_store: SharedStaticMemoryPool,
}

impl UdpTmHandler for StaticUdpTmHandler {
    fn send_tm_to_udp_client(&mut self, socket: &UdpSocket, &recv_addr: &SocketAddr) {
        while let Ok(pus_tm_in_pool) = self.tm_rx.try_recv() {
            let store_lock = self.tm_store.write();
            if store_lock.is_err() {
                warn!("Locking TM store failed");
                continue;
            }
            let mut store_lock = store_lock.unwrap();
            let pg = store_lock.read_with_guard(pus_tm_in_pool.store_addr);
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
    pub tm_rx: mpsc::Receiver<PacketAsVec>,
}

impl UdpTmHandler for DynamicUdpTmHandler {
    fn send_tm_to_udp_client(&mut self, socket: &UdpSocket, recv_addr: &SocketAddr) {
        while let Ok(tm) = self.tm_rx.try_recv() {
            if tm.packet.len() > 9 {
                let service = tm.packet[7];
                let subservice = tm.packet[8];
                info!("Sending PUS TM[{service},{subservice}]")
            } else {
                info!("Sending PUS TM");
            }
            let result = socket.send_to(&tm.packet, recv_addr);
            if let Err(e) = result {
                warn!("Sending TM with UDP socket failed: {e}")
            }
        }
    }
}

pub struct UdpTmtcServer<
    TcSender: PacketSenderRaw<Error = SendError>,
    TmHandler: UdpTmHandler,
    SendError,
> {
    pub udp_tc_server: UdpTcServer<TcSender, SendError>,
    pub tm_handler: TmHandler,
}

impl<
        TcSender: PacketSenderRaw<Error = SendError>,
        TmHandler: UdpTmHandler,
        SendError: Debug + 'static,
    > UdpTmtcServer<TcSender, TmHandler, SendError>
{
    pub fn periodic_operation(&mut self) {
        loop {
            if self.poll_tc_server() == HandlingStatus::Empty {
                break;
            }
        }
        if let Some(recv_addr) = self.udp_tc_server.last_sender() {
            self.tm_handler
                .send_tm_to_udp_client(&self.udp_tc_server.socket, &recv_addr);
        }
    }

    fn poll_tc_server(&mut self) -> HandlingStatus {
        match self.udp_tc_server.try_recv_tc() {
            Ok(_) => HandlingStatus::HandledOne,
            Err(e) => {
                match e {
                    ReceiveResult::NothingReceived => (),
                    ReceiveResult::Io(e) => {
                        warn!("IO error {e}");
                    }
                    ReceiveResult::Send(send_error) => {
                        warn!("send error {send_error:?}");
                    }
                }
                HandlingStatus::Empty
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::VecDeque,
        net::IpAddr,
        sync::{Arc, Mutex},
    };

    use satrs::{
        spacepackets::{
            ecss::{tc::PusTcCreator, WritablePusPacket},
            SpHeader,
        },
        tmtc::PacketSenderRaw,
        ComponentId,
    };
    use satrs_example::config::{components, OBSW_SERVER_ADDR};

    use super::*;

    const UDP_SERVER_ID: ComponentId = 0x05;

    #[derive(Default, Debug)]
    pub struct TestSender {
        tc_vec: RefCell<VecDeque<PacketAsVec>>,
    }

    impl PacketSenderRaw for TestSender {
        type Error = ();

        fn send_packet(&self, sender_id: ComponentId, tc_raw: &[u8]) -> Result<(), Self::Error> {
            let mut mut_queue = self.tc_vec.borrow_mut();
            mut_queue.push_back(PacketAsVec::new(sender_id, tc_raw.to_vec()));
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
        let test_receiver = TestSender::default();
        // let tc_queue = test_receiver.tc_vec.clone();
        let udp_tc_server =
            UdpTcServer::new(UDP_SERVER_ID, sock_addr, 2048, test_receiver).unwrap();
        let tm_handler = TestTmHandler::default();
        let tm_handler_calls = tm_handler.addrs_to_send_to.clone();
        let mut udp_dyn_server = UdpTmtcServer {
            udp_tc_server,
            tm_handler,
        };
        udp_dyn_server.periodic_operation();
        let queue = udp_dyn_server.udp_tc_server.tc_sender.tc_vec.borrow();
        assert!(queue.is_empty());
        assert!(tm_handler_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn test_transactions() {
        let sock_addr = SocketAddr::new(IpAddr::V4(OBSW_SERVER_ADDR), 0);
        let test_receiver = TestSender::default();
        // let tc_queue = test_receiver.tc_vec.clone();
        let udp_tc_server =
            UdpTcServer::new(UDP_SERVER_ID, sock_addr, 2048, test_receiver).unwrap();
        let server_addr = udp_tc_server.socket.local_addr().unwrap();
        let tm_handler = TestTmHandler::default();
        let tm_handler_calls = tm_handler.addrs_to_send_to.clone();
        let mut udp_dyn_server = UdpTmtcServer {
            udp_tc_server,
            tm_handler,
        };
        let sph = SpHeader::new_for_unseg_tc(components::Apid::GenericPus as u16, 0, 0);
        let ping_tc = PusTcCreator::new_simple(sph, 17, 1, &[], true)
            .to_vec()
            .unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").expect("Connecting to UDP server failed");
        let client_addr = client.local_addr().unwrap();
        client.connect(server_addr).unwrap();
        client.send(&ping_tc).unwrap();
        udp_dyn_server.periodic_operation();
        {
            let mut queue = udp_dyn_server.udp_tc_server.tc_sender.tc_vec.borrow_mut();
            assert!(!queue.is_empty());
            let packet_with_sender = queue.pop_front().unwrap();
            assert_eq!(packet_with_sender.packet, ping_tc);
            assert_eq!(packet_with_sender.sender_id, UDP_SERVER_ID);
        }

        {
            let mut tm_handler_calls = tm_handler_calls.lock().unwrap();
            assert!(!tm_handler_calls.is_empty());
            assert_eq!(tm_handler_calls.len(), 1);
            let received_addr = tm_handler_calls.pop_front().unwrap();
            assert_eq!(received_addr, client_addr);
        }
        udp_dyn_server.periodic_operation();
        let queue = udp_dyn_server.udp_tc_server.tc_sender.tc_vec.borrow();
        assert!(queue.is_empty());
        drop(queue);
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
