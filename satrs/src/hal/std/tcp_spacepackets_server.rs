use alloc::sync::Arc;
use core::{sync::atomic::AtomicBool, time::Duration};
use delegate::delegate;
use mio::net::{TcpListener, TcpStream};
use std::{io::Write, net::SocketAddr};

use crate::{
    encoding::{ccsds::SpacePacketValidator, parse_buffer_for_ccsds_space_packets},
    tmtc::{PacketSenderRaw, PacketSource},
    ComponentId,
};

use super::tcp_server::{
    ConnectionResult, HandledConnectionHandler, HandledConnectionInfo, ServerConfig, TcpTcParser,
    TcpTmSender, TcpTmtcError, TcpTmtcGenericServer,
};

impl<T: SpacePacketValidator, TmError, TcError: 'static> TcpTcParser<TmError, TcError> for T {
    fn handle_tc_parsing(
        &mut self,
        tc_buffer: &mut [u8],
        sender_id: ComponentId,
        tc_sender: &(impl PacketSenderRaw<Error = TcError> + ?Sized),
        conn_result: &mut HandledConnectionInfo,
        current_write_idx: usize,
        next_write_idx: &mut usize,
    ) -> Result<(), TcpTmtcError<TmError, TcError>> {
        // Reader vec full, need to parse for packets.
        let parse_result = parse_buffer_for_ccsds_space_packets(
            &tc_buffer[..current_write_idx],
            self,
            sender_id,
            tc_sender,
        )
        .map_err(|e| TcpTmtcError::TcError(e))?;
        if let Some(broken_tail_start) = parse_result.incomplete_tail_start {
            // Copy broken tail to front of buffer.
            tc_buffer.copy_within(broken_tail_start..current_write_idx, 0);
            *next_write_idx = current_write_idx - broken_tail_start;
        }
        conn_result.num_received_tcs += parse_result.packets_found;
        Ok(())
    }
}

/// Concrete [TcpTmSender] implementation for the [TcpSpacepacketsServer].
#[derive(Default)]
pub struct SpacepacketsTmSender {}

impl<TmError, TcError> TcpTmSender<TmError, TcError> for SpacepacketsTmSender {
    fn handle_tm_sending(
        &mut self,
        tm_buffer: &mut [u8],
        tm_source: &mut (impl PacketSource<Error = TmError> + ?Sized),
        conn_result: &mut HandledConnectionInfo,
        stream: &mut TcpStream,
    ) -> Result<bool, TcpTmtcError<TmError, TcError>> {
        let mut tm_was_sent = false;
        loop {
            // Write TM until TM source is exhausted. For now, there is no limit for the amount
            // of TM written this way.
            let read_tm_len = tm_source
                .retrieve_packet(tm_buffer)
                .map_err(|e| TcpTmtcError::TmError(e))?;

            if read_tm_len == 0 {
                return Ok(tm_was_sent);
            }
            tm_was_sent = true;
            conn_result.num_sent_tms += 1;

            stream.write_all(&tm_buffer[..read_tm_len])?;
        }
    }
}

/// TCP TMTC server implementation for exchange of tightly stuffed
/// [CCSDS space packets](https://public.ccsds.org/Pubs/133x0b2e1.pdf).
///
/// This serves only works if
/// [CCSDS 133.0-B-2 space packets](https://public.ccsds.org/Pubs/133x0b2e1.pdf) are the only
/// packet type being exchanged. It uses the CCSDS space packet header [spacepackets::SpHeader] and
/// a user specified [SpacePacketValidator] to determine the space packets relevant for further
/// processing.
///
/// ## Example
///
/// The [TCP server integration tests](https://egit.irs.uni-stuttgart.de/rust/sat-rs/src/branch/main/satrs/tests/tcp_servers.rs)
/// also serves as the example application for this module.
pub struct TcpSpacepacketsServer<
    TmSource: PacketSource<Error = TmError>,
    TcSender: PacketSenderRaw<Error = SendError>,
    Validator: SpacePacketValidator,
    HandledConnection: HandledConnectionHandler,
    TmError,
    SendError: 'static,
> {
    pub generic_server: TcpTmtcGenericServer<
        TmSource,
        TcSender,
        SpacepacketsTmSender,
        Validator,
        HandledConnection,
        TmError,
        SendError,
    >,
}

impl<
        TmSource: PacketSource<Error = TmError>,
        TcSender: PacketSenderRaw<Error = TcError>,
        Validator: SpacePacketValidator,
        HandledConnection: HandledConnectionHandler,
        TmError: 'static,
        TcError: 'static,
    > TcpSpacepacketsServer<TmSource, TcSender, Validator, HandledConnection, TmError, TcError>
{
    ///
    /// ## Parameter
    ///
    /// * `cfg` - Configuration of the server.
    /// * `tm_source` - Generic TM source used by the server to pull telemetry packets which are
    ///   then sent back to the client.
    /// * `tc_sender` - Any received telecommands which were decoded successfully will be
    ///   forwarded using this [PacketSenderRaw].
    /// * `validator` - Used to determine the space packets relevant for further processing and
    ///   to detect broken space packets.
    /// * `handled_connection_hook` - Called to notify the user about a succesfully handled
    ///   connection.
    /// * `stop_signal` - Can be used to shut down the TCP server even for longer running
    ///   connections.
    pub fn new(
        cfg: ServerConfig,
        tm_source: TmSource,
        tc_sender: TcSender,
        validator: Validator,
        handled_connection_hook: HandledConnection,
        stop_signal: Option<Arc<AtomicBool>>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            generic_server: TcpTmtcGenericServer::new(
                cfg,
                validator,
                SpacepacketsTmSender::default(),
                tm_source,
                tc_sender,
                handled_connection_hook,
                stop_signal,
            )?,
        })
    }

    delegate! {
        to self.generic_server {
            pub fn listener(&mut self) -> &mut TcpListener;

            /// Can be used to retrieve the local assigned address of the TCP server. This is especially
            /// useful if using the port number 0 for OS auto-assignment.
            pub fn local_addr(&self) -> std::io::Result<SocketAddr>;

            /// Delegation to the [TcpTmtcGenericServer::handle_all_connections] call.
            pub fn handle_all_connections(
                &mut self,
                poll_timeout: Option<Duration>
            ) -> Result<ConnectionResult, TcpTmtcError<TmError, TcError>>;
        }
    }
}

#[cfg(test)]
mod tests {
    use core::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };
    #[allow(unused_imports)]
    use std::println;
    use std::{
        io::{Read, Write},
        net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
        sync::mpsc,
        thread,
    };

    use alloc::sync::Arc;
    use hashbrown::HashSet;
    use spacepackets::{
        ecss::{tc::PusTcCreator, WritablePusPacket},
        CcsdsPacket, PacketId, SpHeader,
    };

    use crate::{
        encoding::ccsds::{SpValidity, SpacePacketValidator},
        hal::std::tcp_server::{
            tests::{ConnectionFinishedHandler, SyncTmSource},
            ConnectionResult, ServerConfig,
        },
        queue::GenericSendError,
        tmtc::PacketAsVec,
        ComponentId,
    };

    use super::TcpSpacepacketsServer;

    const TCP_SERVER_ID: ComponentId = 0x05;
    const TEST_APID_0: u16 = 0x02;
    const TEST_PACKET_ID_0: PacketId = PacketId::new_for_tc(true, TEST_APID_0);
    const TEST_APID_1: u16 = 0x10;
    const TEST_PACKET_ID_1: PacketId = PacketId::new_for_tc(true, TEST_APID_1);

    #[derive(Default)]
    pub struct SimpleValidator(pub HashSet<PacketId>);

    impl SpacePacketValidator for SimpleValidator {
        fn validate(&self, sp_header: &SpHeader, _raw_buf: &[u8]) -> SpValidity {
            if self.0.contains(&sp_header.packet_id()) {
                return SpValidity::Valid;
            }
            // Simple case: Assume that the interface always contains valid space packets.
            SpValidity::Skip
        }
    }

    fn generic_tmtc_server(
        addr: &SocketAddr,
        tc_sender: mpsc::Sender<PacketAsVec>,
        tm_source: SyncTmSource,
        validator: SimpleValidator,
        stop_signal: Option<Arc<AtomicBool>>,
    ) -> TcpSpacepacketsServer<
        SyncTmSource,
        mpsc::Sender<PacketAsVec>,
        SimpleValidator,
        ConnectionFinishedHandler,
        (),
        GenericSendError,
    > {
        TcpSpacepacketsServer::new(
            ServerConfig::new(TCP_SERVER_ID, *addr, Duration::from_millis(2), 1024, 1024),
            tm_source,
            tc_sender,
            validator,
            ConnectionFinishedHandler::default(),
            stop_signal,
        )
        .expect("TCP server generation failed")
    }

    #[test]
    fn test_basic_tc_only() {
        let auto_port_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
        let (tc_sender, tc_receiver) = mpsc::channel();
        let tm_source = SyncTmSource::default();
        let mut validator = SimpleValidator::default();
        validator.0.insert(TEST_PACKET_ID_0);
        let mut tcp_server = generic_tmtc_server(
            &auto_port_addr,
            tc_sender.clone(),
            tm_source,
            validator,
            None,
        );
        let dest_addr = tcp_server
            .local_addr()
            .expect("retrieving dest addr failed");
        let conn_handled: Arc<AtomicBool> = Default::default();
        let set_if_done = conn_handled.clone();
        // Call the connection handler in separate thread, does block.
        thread::spawn(move || {
            let result = tcp_server.handle_all_connections(Some(Duration::from_millis(100)));
            if result.is_err() {
                panic!("handling connection failed: {:?}", result.unwrap_err());
            }
            let conn_result = result.unwrap();
            matches!(conn_result, ConnectionResult::HandledConnections(1));
            tcp_server
                .generic_server
                .finished_handler
                .check_last_connection(0, 1);
            tcp_server
                .generic_server
                .finished_handler
                .check_no_connections_left();
            set_if_done.store(true, Ordering::Relaxed);
        });
        let ping_tc =
            PusTcCreator::new_simple(SpHeader::new_from_apid(TEST_APID_0), 17, 1, &[], true);
        let tc_0 = ping_tc.to_vec().expect("packet generation failed");
        let mut stream = TcpStream::connect(dest_addr).expect("connecting to TCP server failed");
        stream
            .write_all(&tc_0)
            .expect("writing to TCP server failed");
        drop(stream);

        // A certain amount of time is allowed for the transaction to complete.
        for _ in 0..3 {
            if !conn_handled.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
        }
        if !conn_handled.load(Ordering::Relaxed) {
            panic!("connection was not handled properly");
        }
        let packet = tc_receiver.try_recv().expect("receiving TC failed");
        assert_eq!(packet.packet, tc_0);
        matches!(tc_receiver.try_recv(), Err(mpsc::TryRecvError::Empty));
    }

    #[test]
    fn test_multi_tc_multi_tm() {
        let auto_port_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
        let (tc_sender, tc_receiver) = mpsc::channel();
        let mut tm_source = SyncTmSource::default();

        // Add telemetry
        let mut total_tm_len = 0;
        let verif_tm =
            PusTcCreator::new_simple(SpHeader::new_from_apid(TEST_APID_0), 1, 1, &[], true);
        let tm_0 = verif_tm.to_vec().expect("writing packet failed");
        total_tm_len += tm_0.len();
        tm_source.add_tm(&tm_0);
        let verif_tm =
            PusTcCreator::new_simple(SpHeader::new_from_apid(TEST_APID_1), 1, 3, &[], true);
        let tm_1 = verif_tm.to_vec().expect("writing packet failed");
        total_tm_len += tm_1.len();
        tm_source.add_tm(&tm_1);

        // Set up server
        let mut validator = SimpleValidator::default();
        validator.0.insert(TEST_PACKET_ID_0);
        validator.0.insert(TEST_PACKET_ID_1);
        let mut tcp_server = generic_tmtc_server(
            &auto_port_addr,
            tc_sender.clone(),
            tm_source,
            validator,
            None,
        );
        let dest_addr = tcp_server
            .local_addr()
            .expect("retrieving dest addr failed");
        let conn_handled: Arc<AtomicBool> = Default::default();
        let set_if_done = conn_handled.clone();

        // Call the connection handler in separate thread, does block.
        thread::spawn(move || {
            let result = tcp_server.handle_all_connections(Some(Duration::from_millis(100)));
            if result.is_err() {
                panic!("handling connection failed: {:?}", result.unwrap_err());
            }
            let conn_result = result.unwrap();
            matches!(conn_result, ConnectionResult::HandledConnections(1));
            tcp_server
                .generic_server
                .finished_handler
                .check_last_connection(2, 2);
            tcp_server
                .generic_server
                .finished_handler
                .check_no_connections_left();
            set_if_done.store(true, Ordering::Relaxed);
        });
        let mut stream = TcpStream::connect(dest_addr).expect("connecting to TCP server failed");
        stream
            .set_read_timeout(Some(Duration::from_millis(10)))
            .expect("setting reas timeout failed");

        // Send telecommands
        let ping_tc =
            PusTcCreator::new_simple(SpHeader::new_from_apid(TEST_APID_0), 17, 1, &[], true);
        let tc_0 = ping_tc.to_vec().expect("ping tc creation failed");
        stream
            .write_all(&tc_0)
            .expect("writing to TCP server failed");
        let action_tc =
            PusTcCreator::new_simple(SpHeader::new_from_apid(TEST_APID_1), 8, 0, &[], true);
        let tc_1 = action_tc.to_vec().expect("action tc creation failed");
        stream
            .write_all(&tc_1)
            .expect("writing to TCP server failed");

        // Done with writing.
        stream
            .shutdown(std::net::Shutdown::Write)
            .expect("shutting down write failed");
        let mut read_buf: [u8; 32] = [0; 32];
        let mut current_idx = 0;
        let mut read_len_total = 0;
        // Timeout ensures this does not block forever.
        while read_len_total < total_tm_len {
            let read_len = stream
                .read(&mut read_buf[current_idx..])
                .expect("read failed");
            current_idx += read_len;
            read_len_total += read_len;
        }
        drop(stream);
        assert_eq!(read_buf[..tm_0.len()], tm_0);
        assert_eq!(read_buf[tm_0.len()..tm_0.len() + tm_1.len()], tm_1);

        // A certain amount of time is allowed for the transaction to complete.
        for _ in 0..3 {
            if !conn_handled.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
        }
        if !conn_handled.load(Ordering::Relaxed) {
            panic!("connection was not handled properly");
        }
        // Check that TC has arrived.
        let packet_0 = tc_receiver.try_recv().expect("receiving TC failed");
        assert_eq!(packet_0.packet, tc_0);
        let packet_1 = tc_receiver.try_recv().expect("receiving TC failed");
        assert_eq!(packet_1.packet, tc_1);
        matches!(tc_receiver.try_recv(), Err(mpsc::TryRecvError::Empty));
    }
}
