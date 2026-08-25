//! # Serial packet transport with COBS encoding.
use std::time::Duration;

use cobs::CobsDecoderOwned;

use crate::transport::PacketTransport;

/// Packet transport via a serial interface with COBS encoding.
pub struct PacketTransportSerialCobs {
    /// Underlying serial port.
    pub serial: Box<dyn serialport::SerialPort>,
    /// Enables/disables logging of decoding errors.
    pub log_decoding_errors: bool,
    reception_buffer: [u8; 1024],
    decoder: cobs::CobsDecoderOwned,
}

impl PacketTransportSerialCobs {
    /// Constructor which constructs the [serialport::SerialPort] and [cobs::CobsDecoderOwned] from
    /// the passed parameters.
    ///
    /// The `max_rx_packet_size` parameter defines the expected maximum size of a received packet.
    /// On non-linux platforms, the serial timeout parameter has to be specified as well.
    pub fn new_from_params(
        port_name: &str,
        baud_rate: u32,
        max_rx_packet_size: usize,
    ) -> Result<Self, std::io::Error> {
        let serial = serialport::new(port_name, baud_rate).open_native()?;
        // Not merged yet in upstream..
        //#[cfg(target_os = "linux")]
        //serial.set_read_mode(ReadMode::Immediate)?;
        Ok(Self::new(
            Box::new(serial),
            CobsDecoderOwned::new(max_rx_packet_size),
        ))
    }

    /// Generic constructor.
    pub fn new(serial: Box<dyn serialport::SerialPort>, decoder: cobs::CobsDecoderOwned) -> Self {
        Self {
            serial,
            decoder,
            reception_buffer: [0u8; 1024],
            log_decoding_errors: true,
        }
    }

    /// Set the serial port timeout.
    pub fn set_serial_timeout(&mut self, timeout: Duration) -> Result<(), serialport::Error> {
        self.serial.set_timeout(timeout)
    }

    /// Send a packet.
    ///
    /// It encodes the packet using COBS encoding before sending it over the serial port.
    pub fn send(&mut self, packet: &[u8]) -> Result<(), super::SendError> {
        let encoded = cobs::encode_vec_including_sentinels(packet);
        log::debug!("sending COBS encoded packet: {:?}", encoded);
        self.serial.write_all(&encoded)?;
        // This is required to avoid timeout errors on bursty data.
        self.serial.flush()?;
        Ok(())
    }

    /// Received packets.
    ///
    /// This function pulls bytes from the serial port and feeds them into the COBS decoder.
    /// For each received packet, the closure will be called with the decoded packet as an argument.
    /// The function will return the number of received packets.
    pub fn receive<F: FnMut(&[u8])>(&mut self, mut f: F) -> Result<usize, super::ReceiveError> {
        let mut decoded_packets = 0;
        loop {
            match self.serial.read(&mut self.reception_buffer) {
                Ok(read_bytes) => {
                    if read_bytes == 0 {
                        break;
                    }
                    for byte in self.reception_buffer[..read_bytes].iter() {
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
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock
                    {
                        break;
                    }
                    return Err(super::ReceiveError::Io(e));
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
}

impl PacketTransport for PacketTransportSerialCobs {
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
    use std::time::Duration;

    use serialport::SerialPort;

    #[derive(Debug)]
    pub struct SerialPortMock {
        baud: u32,
        pub tx_data: std::sync::mpsc::Sender<Vec<u8>>,
        pub rx_data: std::sync::mpsc::Receiver<Vec<u8>>,
    }

    impl std::io::Write for SerialPortMock {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.tx_data.send(buf.to_vec()).unwrap();
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl std::io::Read for SerialPortMock {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.rx_data.try_recv() {
                Ok(packet) => {
                    buf[0..packet.len()].copy_from_slice(&packet);
                    Ok(packet.len())
                }
                Err(e) => match e {
                    std::sync::mpsc::TryRecvError::Empty => Ok(0),
                    std::sync::mpsc::TryRecvError::Disconnected => panic!("sender disconnected"),
                },
            }
        }
    }

    impl SerialPort for SerialPortMock {
        fn name(&self) -> Option<String> {
            Some("mock".into())
        }

        fn baud_rate(&self) -> serialport::Result<u32> {
            Ok(115200)
        }

        fn data_bits(&self) -> serialport::Result<serialport::DataBits> {
            Ok(serialport::DataBits::Eight)
        }

        fn flow_control(&self) -> serialport::Result<serialport::FlowControl> {
            Ok(serialport::FlowControl::None)
        }

        fn parity(&self) -> serialport::Result<serialport::Parity> {
            Ok(serialport::Parity::None)
        }

        fn stop_bits(&self) -> serialport::Result<serialport::StopBits> {
            Ok(serialport::StopBits::One)
        }

        fn timeout(&self) -> std::time::Duration {
            Duration::from_millis(0)
        }

        fn set_baud_rate(&mut self, baud_rate: u32) -> serialport::Result<()> {
            self.baud = baud_rate;
            Ok(())
        }

        fn set_data_bits(&mut self, _data_bits: serialport::DataBits) -> serialport::Result<()> {
            Ok(())
        }

        fn set_flow_control(
            &mut self,
            _flow_control: serialport::FlowControl,
        ) -> serialport::Result<()> {
            Ok(())
        }

        fn set_parity(&mut self, _parity: serialport::Parity) -> serialport::Result<()> {
            Ok(())
        }

        fn set_stop_bits(&mut self, _stop_bits: serialport::StopBits) -> serialport::Result<()> {
            Ok(())
        }

        fn set_timeout(&mut self, _timeout: std::time::Duration) -> serialport::Result<()> {
            Ok(())
        }

        fn write_request_to_send(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }

        fn write_data_terminal_ready(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }

        fn read_clear_to_send(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }

        fn read_data_set_ready(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }

        fn read_ring_indicator(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }

        fn read_carrier_detect(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }

        fn bytes_to_read(&self) -> serialport::Result<u32> {
            Err(serialport::Error::new(
                serialport::ErrorKind::NoDevice,
                "Mock does not support bytes_to_read",
            ))
        }

        fn bytes_to_write(&self) -> serialport::Result<u32> {
            Ok(0)
        }

        fn clear(&self, _buffer_to_clear: serialport::ClearBuffer) -> serialport::Result<()> {
            Ok(())
        }

        fn try_clone(&self) -> serialport::Result<Box<dyn SerialPort>> {
            serialport::Result::Err(serialport::Error::new(
                serialport::ErrorKind::NoDevice,
                "Mock clone not supported",
            ))
        }

        fn set_break(&self) -> serialport::Result<()> {
            Ok(())
        }

        fn clear_break(&self) -> serialport::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn basic_tx_test() {
        let (tx_tx, tx_rx) = std::sync::mpsc::channel();
        let (_rx_tx, rx_rx) = std::sync::mpsc::channel();
        let mut transport = PacketTransportSerialCobs::new(
            Box::new(SerialPortMock {
                baud: 115200,
                tx_data: tx_tx,
                rx_data: rx_rx,
            }),
            CobsDecoderOwned::new(128),
        );
        let sent_data = [1, 2, 3, 4];
        transport.send(&sent_data).unwrap();
        let encoded_data = tx_rx.recv().unwrap();
        assert_eq!(
            encoded_data,
            cobs::encode_vec_including_sentinels(&sent_data)
        );
    }

    #[test]
    fn basic_rx_test() {
        let (tx_tx, _tx_rx) = std::sync::mpsc::channel();
        let (rx_tx, rx_rx) = std::sync::mpsc::channel();
        let mut transport = PacketTransportSerialCobs::new(
            Box::new(SerialPortMock {
                baud: 115200,
                tx_data: tx_tx,
                rx_data: rx_rx,
            }),
            CobsDecoderOwned::new(128),
        );
        let rx_data = [1, 2, 3, 4];
        rx_tx
            .send(cobs::encode_vec_including_sentinels(&rx_data))
            .unwrap();
        assert_eq!(
            transport
                .receive(|packet| {
                    assert_eq!(packet, &rx_data);
                })
                .unwrap(),
            1
        );
    }
}
