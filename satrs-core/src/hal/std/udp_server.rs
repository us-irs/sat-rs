//! Generic UDP TC server.
use crate::tmtc::{ReceivesTc, ReceivesTcCore};
use std::boxed::Box;
use std::io::{Error, ErrorKind};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::vec;
use std::vec::Vec;

/// This UDP server can be used to receive CCSDS space packet telecommands or any other telecommand
/// format.
///
/// It caches all received telecomands into a vector. The maximum expected telecommand size should
/// be declared upfront. This avoids dynamic allocation during run-time. The user can specify a TC
/// receiver in form of a special trait object which implements [ReceivesTc]. Please note that the
/// receiver should copy out the received data if it the data is required past the
/// [ReceivesTcCore::pass_tc] call.
///
/// # Examples
///
/// ```
/// use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
/// use spacepackets::ecss::SerializablePusPacket;
/// use satrs_core::hal::host::udp_server::UdpTcServer;
/// use satrs_core::tmtc::{ReceivesTc, ReceivesTcCore};
/// use spacepackets::SpHeader;
/// use spacepackets::ecss::tc::PusTcCreator;
///
/// #[derive (Default)]
/// struct PingReceiver {}
/// impl ReceivesTcCore for PingReceiver {
///    type Error = ();
///    fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
///         assert_eq!(tc_raw.len(), 13);
///         Ok(())
///     }
/// }
///
/// let mut buf = [0; 32];
/// let dest_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 7777);
/// let ping_receiver = PingReceiver::default();
/// let mut udp_tc_server = UdpTcServer::new(dest_addr, 2048, Box::new(ping_receiver))
///       .expect("Creating UDP TMTC server failed");
/// let mut sph = SpHeader::tc_unseg(0x02, 0, 0).unwrap();
/// let pus_tc = PusTcCreator::new_simple(&mut sph, 17, 1, None, true);
/// let len = pus_tc
///     .write_to_bytes(&mut buf)
///     .expect("Error writing PUS TC packet");
/// assert_eq!(len, 13);
/// let client = UdpSocket::bind("127.0.0.1:7778").expect("Connecting to UDP server failed");
/// client
///     .send_to(&buf[0..len], dest_addr)
///     .expect("Error sending PUS TC via UDP");
/// ```
///
/// The [fsrc-example crate](https://egit.irs.uni-stuttgart.de/rust/fsrc-launchpad/src/branch/main/fsrc-example)
/// server code also includes
/// [example code](https://egit.irs.uni-stuttgart.de/rust/fsrc-launchpad/src/branch/main/fsrc-example/src/bin/obsw/tmtc.rs)
/// on how to use this TC server. It uses the server to receive PUS telecommands on a specific port
/// and then forwards them to a generic CCSDS packet receiver.
pub struct UdpTcServer<E> {
    pub socket: UdpSocket,
    recv_buf: Vec<u8>,
    sender_addr: Option<SocketAddr>,
    tc_receiver: Box<dyn ReceivesTc<Error = E>>,
}

#[derive(Debug)]
pub enum ReceiveResult<E> {
    NothingReceived,
    IoError(Error),
    ReceiverError(E),
}

impl<E> From<Error> for ReceiveResult<E> {
    fn from(e: Error) -> Self {
        ReceiveResult::IoError(e)
    }
}

impl<E: PartialEq> PartialEq for ReceiveResult<E> {
    fn eq(&self, other: &Self) -> bool {
        use ReceiveResult::*;
        match (self, other) {
            (IoError(ref e), IoError(ref other_e)) => e.kind() == other_e.kind(),
            (NothingReceived, NothingReceived) => true,
            (ReceiverError(e), ReceiverError(other_e)) => e == other_e,
            _ => false,
        }
    }
}

impl<E: Eq + PartialEq> Eq for ReceiveResult<E> {}

impl<E: 'static> ReceivesTcCore for UdpTcServer<E> {
    type Error = E;

    fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
        self.tc_receiver.pass_tc(tc_raw)
    }
}

impl<E: 'static> UdpTcServer<E> {
    pub fn new<A: ToSocketAddrs>(
        addr: A,
        max_recv_size: usize,
        tc_receiver: Box<dyn ReceivesTc<Error = E>>,
    ) -> Result<Self, Error> {
        let server = Self {
            socket: UdpSocket::bind(addr)?,
            recv_buf: vec![0; max_recv_size],
            sender_addr: None,
            tc_receiver,
        };
        server.socket.set_nonblocking(true)?;
        Ok(server)
    }

    pub fn try_recv_tc(&mut self) -> Result<(usize, SocketAddr), ReceiveResult<E>> {
        let res = match self.socket.recv_from(&mut self.recv_buf) {
            Ok(res) => res,
            Err(e) => {
                return if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut {
                    Err(ReceiveResult::NothingReceived)
                } else {
                    Err(e.into())
                }
            }
        };
        let (num_bytes, from) = res;
        self.sender_addr = Some(from);
        self.tc_receiver
            .pass_tc(&self.recv_buf[0..num_bytes])
            .map_err(|e| ReceiveResult::ReceiverError(e))?;
        Ok(res)
    }

    pub fn last_sender(&self) -> Option<SocketAddr> {
        self.sender_addr
    }
}

#[cfg(test)]
mod tests {
    use crate::hal::host::udp_server::{ReceiveResult, UdpTcServer};
    use crate::tmtc::ReceivesTcCore;
    use spacepackets::ecss::tc::PusTcCreator;
    use spacepackets::ecss::SerializablePusPacket;
    use spacepackets::SpHeader;
    use std::boxed::Box;
    use std::collections::VecDeque;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
    use std::vec::Vec;

    fn is_send<T: Send>(_: &T) {}

    #[derive(Default)]
    struct PingReceiver {
        pub sent_cmds: VecDeque<Vec<u8>>,
    }

    impl ReceivesTcCore for PingReceiver {
        type Error = ();

        fn pass_tc(&mut self, tc_raw: &[u8]) -> Result<(), Self::Error> {
            let mut sent_data = Vec::new();
            sent_data.extend_from_slice(tc_raw);
            self.sent_cmds.push_back(sent_data);
            Ok(())
        }
    }

    #[test]
    fn basic_test() {
        let mut buf = [0; 32];
        let dest_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 7777);
        let ping_receiver = PingReceiver::default();
        is_send(&ping_receiver);
        let mut udp_tc_server = UdpTcServer::new(dest_addr, 2048, Box::new(ping_receiver))
            .expect("Creating UDP TMTC server failed");
        is_send(&udp_tc_server);
        let mut sph = SpHeader::tc_unseg(0x02, 0, 0).unwrap();
        let pus_tc = PusTcCreator::new_simple(&mut sph, 17, 1, None, true);
        let len = pus_tc
            .write_to_bytes(&mut buf)
            .expect("Error writing PUS TC packet");
        let client = UdpSocket::bind("127.0.0.1:7778").expect("Connecting to UDP server failed");
        client
            .send_to(&buf[0..len], dest_addr)
            .expect("Error sending PUS TC via UDP");
        let local_addr = client.local_addr().unwrap();
        udp_tc_server
            .try_recv_tc()
            .expect("Error receiving sent telecommand");
        assert_eq!(
            udp_tc_server.last_sender().expect("No sender set"),
            local_addr
        );
        let ping_receiver: &mut PingReceiver = udp_tc_server.tc_receiver.downcast_mut().unwrap();
        assert_eq!(ping_receiver.sent_cmds.len(), 1);
        let sent_cmd = ping_receiver.sent_cmds.pop_front().unwrap();
        assert_eq!(sent_cmd, buf[0..len]);
    }

    #[test]
    fn test_nothing_received() {
        let dest_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 7779);
        let ping_receiver = PingReceiver::default();
        let mut udp_tc_server = UdpTcServer::new(dest_addr, 2048, Box::new(ping_receiver))
            .expect("Creating UDP TMTC server failed");
        let res = udp_tc_server.try_recv_tc();
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err, ReceiveResult::NothingReceived);
    }
}
