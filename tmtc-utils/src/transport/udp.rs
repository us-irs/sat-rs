//! # Packet transport via UDP

use std::io::ErrorKind;
use std::net::ToSocketAddrs;

use crate::transport::PacketTransport;

/// Generic packet transport via UDP.
///
/// Currently only allows a maxium packet size of 4096.
pub struct PacketTransportUdp {
    /// Underlying UDP socket.
    pub socket: std::net::UdpSocket,
    target: std::net::SocketAddr,
    reception_buffer: [u8; 4096],
}

impl PacketTransportUdp {
    /// Generic constructor.
    ///
    /// The `socket` parameter is the underlying UDP stream which should already be connected.
    /// It will be set non-blocking by the construtor.
    pub fn new(
        socket: std::net::UdpSocket,
        target: std::net::SocketAddr,
    ) -> Result<Self, std::io::Error> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            target,
            reception_buffer: [0u8; 4096],
        })
    }

    /// Update default target.
    pub fn set_default_target(&mut self, target: std::net::SocketAddr) {
        self.target = target;
    }

    /// Send a packet to the target address specified in the constructor.
    pub fn send(&mut self, packet: &[u8]) -> Result<(), super::SendError> {
        self.socket
            .send_to(packet, self.target)
            .map_err(super::SendError::Io)?;
        Ok(())
    }

    /// Send packet to a specific address.
    pub fn send_to<A: ToSocketAddrs>(
        &mut self,
        packet: &[u8],
        addr: A,
    ) -> Result<(), super::SendError> {
        self.socket
            .send_to(packet, addr)
            .map_err(super::SendError::Io)?;
        Ok(())
    }

    /// Receive packets and call the provided callback for each received packet.
    pub fn receive<F: FnMut(&[u8])>(&mut self, mut f: F) -> Result<usize, super::ReceiveError> {
        let mut packets_received = 0;
        loop {
            match self.socket.recv_from(&mut self.reception_buffer) {
                Ok((bytes, _)) => {
                    packets_received += 1;
                    f(&self.reception_buffer[..bytes]);
                }
                Err(e) => {
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut {
                        break;
                    }
                    log::error!("UDP reception error: {e}");
                }
            }
        }
        Ok(packets_received)
    }
}

impl PacketTransport for PacketTransportUdp {
    fn send(&mut self, packet: &[u8]) -> Result<(), super::SendError> {
        self.send(packet)
    }

    fn receive<F: FnMut(&[u8])>(&mut self, f: F) -> Result<usize, super::ReceiveError> {
        self.receive(f)
    }

    fn close(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_send_test() {
        let receiver = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

        let receiver_addr = receiver.local_addr().unwrap();
        sender.connect(receiver_addr).unwrap();

        let mut transport = PacketTransportUdp::new(sender, receiver_addr).unwrap();
        let payload = [1u8, 2, 3, 4];

        transport.send(&payload).unwrap();

        let mut buf = [0u8; 16];
        let (len, from) = receiver.recv_from(&mut buf).unwrap();

        assert_eq!(&buf[..len], &payload);
        assert_eq!(from, transport.socket.local_addr().unwrap());
    }

    #[test]
    fn receive_is_non_blocking_when_no_data_is_available() {
        let receiver = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

        let receiver_addr = receiver.local_addr().unwrap();
        let mut transport = PacketTransportUdp::new(sender, receiver_addr).unwrap();

        let start = std::time::Instant::now();
        let mut callback_called = false;

        let packets_received = transport
            .receive(|_| {
                callback_called = true;
            })
            .unwrap();

        let elapsed = start.elapsed();

        assert_eq!(packets_received, 0);
        assert!(!callback_called);
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "receive() took too long for a non-blocking socket: {:?}",
            elapsed
        );
    }

    #[test]
    fn basic_receive_test_single() {
        let plain_sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let transport_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

        let transport_addr = transport_socket.local_addr().unwrap();

        let mut transport = PacketTransportUdp::new(transport_socket, transport_addr).unwrap();

        plain_sender
            .send_to(&[1u8, 2, 3, 4], transport_addr)
            .unwrap();

        let mut received_packets: Vec<Vec<u8>> = Vec::new();
        let packets_received = transport
            .receive(|packet| received_packets.push(packet.to_vec()))
            .unwrap();

        assert_eq!(packets_received, 1);
        assert_eq!(received_packets.len(), 1);
        assert_eq!(received_packets[0], vec![1u8, 2, 3, 4]);
    }

    #[test]
    fn multi_packet_receive_test() {
        let plain_sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let transport_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

        let receiver_addr = transport_socket.local_addr().unwrap();
        let mut transport = PacketTransportUdp::new(transport_socket, receiver_addr).unwrap();

        plain_sender
            .send_to(&[1u8, 2, 3, 4], receiver_addr)
            .unwrap();
        plain_sender
            .send_to(&[5u8, 6, 7, 8], receiver_addr)
            .unwrap();
        plain_sender
            .send_to(&[9u8, 10, 11, 12], receiver_addr)
            .unwrap();

        let mut received_packets: Vec<Vec<u8>> = Vec::new();
        let packets_received = transport
            .receive(|packet| received_packets.push(packet.to_vec()))
            .unwrap();

        assert_eq!(packets_received, 3);
        assert_eq!(received_packets.len(), 3);
        assert_eq!(received_packets[0], vec![1u8, 2, 3, 4]);
        assert_eq!(received_packets[1], vec![5u8, 6, 7, 8]);
        assert_eq!(received_packets[2], vec![9u8, 10, 11, 12]);
    }

    #[test]
    fn send_and_receive_test() {
        // Bind both sockets
        let plain_receiver = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let sender_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

        let receiver_addr = plain_receiver.local_addr().unwrap();

        // Build the transport around the sender socket
        let mut send_transport = PacketTransportUdp::new(sender_socket, receiver_addr).unwrap();

        // Send a packet
        let payload = [1u8, 2, 3, 4];
        send_transport.send(&payload).unwrap();

        // Plain receiver reads what was sent
        let mut buf = [0u8; 4096];
        let (n, src) = plain_receiver.recv_from(&mut buf).unwrap();
        assert_eq!(n, payload.len());
        assert_eq!(&buf[..n], &payload);

        // Plain receiver sends back some test data
        let reply = [5u8, 6, 7, 8];
        plain_receiver.send_to(&reply, src).unwrap();

        // Read the reply via the transport (non-blocking socket, reply should already be in flight)
        let mut received_packets: Vec<Vec<u8>> = Vec::new();
        // Small yield to ensure the reply has arrived on the loopback interface
        std::thread::sleep(std::time::Duration::from_millis(10));

        let packets_received = send_transport
            .receive(|packet| received_packets.push(packet.to_vec()))
            .unwrap();

        assert_eq!(packets_received, 1);
        assert_eq!(received_packets.len(), 1);
        assert_eq!(received_packets[0], reply.to_vec());
    }
}
