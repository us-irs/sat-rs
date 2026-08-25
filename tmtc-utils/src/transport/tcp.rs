//! # Packet transport via TCP with COBS encoding.
use std::{
    io::{Read as _, Write as _},
    time::Duration,
};

use crate::transport::PacketTransport;

/// Packet transport via TCP with COBS encoding.
///
/// Currently only allows a maxium packet size of 4096.
pub struct PacketTransportTcpWithCobs {
    /// Underlying TCP stream.
    pub tcp_stream: std::net::TcpStream,
    /// Can be used to disable logging of decoding errors.
    pub log_decoding_errors: bool,
    /// Decoder object.
    decoder: cobs::CobsDecoderOwned,
    reception_buffer: [u8; 4096],
}

impl PacketTransportTcpWithCobs {
    /// Generic constructor.
    ///
    /// The `tcp_stream` parameter is the underlying TCP stream which should already be connected.
    pub fn new(
        tcp_stream: std::net::TcpStream,
        decoder: cobs::CobsDecoderOwned,
    ) -> std::io::Result<Self> {
        tcp_stream.set_nonblocking(true)?;
        tcp_stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        Ok(Self {
            tcp_stream,
            decoder,
            reception_buffer: [0u8; 4096],
            log_decoding_errors: true,
        })
    }

    /// Send a packet.
    ///
    /// It encodes the packet using COBS encoding before sending it over the TCP stream.
    pub fn send(&mut self, packet: &[u8]) -> Result<(), super::SendError> {
        let cobs_encoded_packet = cobs::encode_vec_including_sentinels(packet);
        self.tcp_stream.write_all(&cobs_encoded_packet)?;
        Ok(())
    }

    /// Received packets.
    ///
    /// This function pulls bytes from the TCP stream and feeds them into the COBS decoder.
    /// For each received packet, the closure will be called with the decoded packet as an argument.
    /// The function will return the number of received packets.
    ///
    /// Please note that this function may block on the TCP stream read call, but it will not
    /// block indifinitely due to the read timeout set on the TCP stream.
    pub fn receive(&mut self, mut f: impl FnMut(&[u8])) -> Result<usize, super::ReceiveError> {
        let mut decoded_packets = 0;
        loop {
            let read_size = self
                .tcp_stream
                .read(&mut self.reception_buffer)
                .unwrap_or(0);
            if read_size == 0 {
                break;
            }
            for byte in &self.reception_buffer[0..read_size] {
                match self.decoder.feed(*byte) {
                    Ok(Some(packet_len)) => {
                        f(&self.decoder.dest()[0..packet_len]);
                        decoded_packets += 1;
                    }
                    Ok(None) => (),
                    Err(e) => self.error_handler(e),
                }
            }
        }
        Ok(decoded_packets)
    }

    fn error_handler(&self, error: cobs::DecodeError) {
        if self.log_decoding_errors {
            log::warn!("COBS decoding error: {:?}", error);
        }
    }

    /// Close the connection by shutting down the TCP stream.
    pub fn close(&mut self) -> std::io::Result<()> {
        self.tcp_stream.shutdown(std::net::Shutdown::Both)
    }
}

impl PacketTransport for PacketTransportTcpWithCobs {
    fn send(&mut self, packet: &[u8]) -> Result<(), super::SendError> {
        self.send(packet)
    }

    fn receive<F: FnMut(&[u8])>(&mut self, f: F) -> Result<usize, super::ReceiveError> {
        self.receive(f)
    }

    fn close(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_send_test() {
        let tcp_server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        tcp_server
            .set_nonblocking(true)
            .expect("failed to set blocking mode");
        let addr = tcp_server.local_addr().unwrap();
        let tcp_client = std::net::TcpStream::connect(addr).unwrap();
        let mut transport =
            PacketTransportTcpWithCobs::new(tcp_client, cobs::CobsDecoderOwned::new(1024)).unwrap();
        let packet = [1, 2, 3, 4];
        transport.send(&packet).unwrap();
        tcp_server
            .accept()
            .map(|(mut stream, _)| {
                let mut buffer = [0u8; 1024];
                let read_size = stream.read(&mut buffer).unwrap();
                let decoded_packet = cobs::decode_vec(&buffer[0..read_size]).unwrap();
                assert_eq!(decoded_packet, packet);
            })
            .unwrap();
    }

    #[test]
    fn basic_receive_test() {
        let tcp_server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        tcp_server
            .set_nonblocking(true)
            .expect("failed to set blocking mode");
        let addr = tcp_server.local_addr().unwrap();
        let tcp_client = std::net::TcpStream::connect(addr).unwrap();
        let mut transport =
            PacketTransportTcpWithCobs::new(tcp_client, cobs::CobsDecoderOwned::new(1024)).unwrap();
        let rx_data = [1, 2, 3, 4];
        let encoded_data = cobs::encode_vec_including_sentinels(&rx_data);
        tcp_server
            .accept()
            .map(|(mut stream, _)| {
                stream.write_all(&encoded_data).unwrap();
            })
            .unwrap();
        transport
            .receive(|packet| {
                assert_eq!(packet, &rx_data);
            })
            .unwrap();
    }
}
