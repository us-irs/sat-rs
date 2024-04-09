use alloc::sync::Arc;
use core::sync::atomic::AtomicBool;
use delegate::delegate;
use std::{
    io::Write,
    net::{SocketAddr, TcpListener, TcpStream},
};

use crate::{
    encoding::parse_buffer_for_ccsds_space_packets,
    tmtc::{ReceivesTc, TmPacketSource},
    ValidatorU16Id,
};

use super::tcp_server::{
    ConnectionResult, ServerConfig, TcpTcParser, TcpTmSender, TcpTmtcError, TcpTmtcGenericServer,
};

/// Concrete [TcpTcParser] implementation for the [TcpSpacepacketsServer].
pub struct SpacepacketsTcParser<PacketIdChecker: ValidatorU16Id> {
    packet_id_lookup: PacketIdChecker,
}

impl<PacketIdChecker: ValidatorU16Id> SpacepacketsTcParser<PacketIdChecker> {
    pub fn new(packet_id_lookup: PacketIdChecker) -> Self {
        Self { packet_id_lookup }
    }
}

impl<TmError, TcError: 'static, PacketIdChecker: ValidatorU16Id> TcpTcParser<TmError, TcError>
    for SpacepacketsTcParser<PacketIdChecker>
{
    fn handle_tc_parsing(
        &mut self,
        tc_buffer: &mut [u8],
        tc_receiver: &mut (impl ReceivesTc<Error = TcError> + ?Sized),
        conn_result: &mut ConnectionResult,
        current_write_idx: usize,
        next_write_idx: &mut usize,
    ) -> Result<(), TcpTmtcError<TmError, TcError>> {
        // Reader vec full, need to parse for packets.
        conn_result.num_received_tcs += parse_buffer_for_ccsds_space_packets(
            &mut tc_buffer[..current_write_idx],
            &self.packet_id_lookup,
            tc_receiver.upcast_mut(),
            next_write_idx,
        )
        .map_err(|e| TcpTmtcError::TcError(e))?;
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
        tm_source: &mut (impl TmPacketSource<Error = TmError> + ?Sized),
        conn_result: &mut ConnectionResult,
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
/// packet type being exchanged. It uses the CCSDS [spacepackets::PacketId] as the packet delimiter
/// and start marker when parsing for packets. The user specifies a set of expected
/// [spacepackets::PacketId]s as part of the server configuration for that purpose.
///
/// ## Example
/// The [TCP server integration tests](https://egit.irs.uni-stuttgart.de/rust/sat-rs/src/branch/main/satrs/tests/tcp_servers.rs)
/// also serves as the example application for this module.
pub struct TcpSpacepacketsServer<
    TmError,
    TcError: 'static,
    TmSource: TmPacketSource<Error = TmError>,
    TcReceiver: ReceivesTc<Error = TcError>,
    PacketIdChecker: ValidatorU16Id,
> {
    generic_server: TcpTmtcGenericServer<
        TmError,
        TcError,
        TmSource,
        TcReceiver,
        SpacepacketsTmSender,
        SpacepacketsTcParser<PacketIdChecker>,
    >,
}

impl<
        TmError: 'static,
        TcError: 'static,
        TmSource: TmPacketSource<Error = TmError>,
        TcReceiver: ReceivesTc<Error = TcError>,
        PacketIdChecker: ValidatorU16Id,
    > TcpSpacepacketsServer<TmError, TcError, TmSource, TcReceiver, PacketIdChecker>
{
    ///
    /// ## Parameter
    ///
    /// * `cfg` - Configuration of the server.
    /// * `tm_source` - Generic TM source used by the server to pull telemetry packets which are
    ///     then sent back to the client.
    /// * `tc_receiver` - Any received telecommands which were decoded successfully will be
    ///     forwarded to this TC receiver.
    /// * `packet_id_lookup` - This lookup table contains the relevant packets IDs for packet
    ///     parsing. This mechanism is used to have a start marker for finding CCSDS packets.
    pub fn new(
        cfg: ServerConfig,
        tm_source: TmSource,
        tc_receiver: TcReceiver,
        packet_id_checker: PacketIdChecker,
        stop_signal: Option<Arc<AtomicBool>>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            generic_server: TcpTmtcGenericServer::new(
                cfg,
                SpacepacketsTcParser::new(packet_id_checker),
                SpacepacketsTmSender::default(),
                tm_source,
                tc_receiver,
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

            /// Delegation to the [TcpTmtcGenericServer::handle_next_connection] call.
            pub fn handle_next_connection(
                &mut self,
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
        thread,
    };

    use alloc::sync::Arc;
    use hashbrown::HashSet;
    use spacepackets::{
        ecss::{tc::PusTcCreator, WritablePusPacket},
        PacketId, SpHeader,
    };

    use crate::hal::std::tcp_server::{
        tests::{SyncTcCacher, SyncTmSource},
        ServerConfig,
    };

    use super::TcpSpacepacketsServer;

    const TEST_APID_0: u16 = 0x02;
    const TEST_PACKET_ID_0: PacketId = PacketId::new_for_tc(true, TEST_APID_0);
    const TEST_APID_1: u16 = 0x10;
    const TEST_PACKET_ID_1: PacketId = PacketId::new_for_tc(true, TEST_APID_1);

    fn generic_tmtc_server(
        addr: &SocketAddr,
        tc_receiver: SyncTcCacher,
        tm_source: SyncTmSource,
        packet_id_lookup: HashSet<PacketId>,
        stop_signal: Option<Arc<AtomicBool>>,
    ) -> TcpSpacepacketsServer<(), (), SyncTmSource, SyncTcCacher, HashSet<PacketId>> {
        TcpSpacepacketsServer::new(
            ServerConfig::new(*addr, Duration::from_millis(2), 1024, 1024),
            tm_source,
            tc_receiver,
            packet_id_lookup,
            stop_signal,
        )
        .expect("TCP server generation failed")
    }

    #[test]
    fn test_basic_tc_only() {
        let auto_port_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
        let tc_receiver = SyncTcCacher::default();
        let tm_source = SyncTmSource::default();
        let mut packet_id_lookup = HashSet::new();
        packet_id_lookup.insert(TEST_PACKET_ID_0);
        let mut tcp_server = generic_tmtc_server(
            &auto_port_addr,
            tc_receiver.clone(),
            tm_source,
            packet_id_lookup,
            None,
        );
        let dest_addr = tcp_server
            .local_addr()
            .expect("retrieving dest addr failed");
        let conn_handled: Arc<AtomicBool> = Default::default();
        let set_if_done = conn_handled.clone();
        // Call the connection handler in separate thread, does block.
        thread::spawn(move || {
            let result = tcp_server.handle_next_connection();
            if result.is_err() {
                panic!("handling connection failed: {:?}", result.unwrap_err());
            }
            let conn_result = result.unwrap();
            assert_eq!(conn_result.num_received_tcs, 1);
            assert_eq!(conn_result.num_sent_tms, 0);
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
        // Check that TC has arrived.
        let mut tc_queue = tc_receiver.tc_queue.lock().unwrap();
        assert_eq!(tc_queue.len(), 1);
        assert_eq!(tc_queue.pop_front().unwrap(), tc_0);
    }

    #[test]
    fn test_multi_tc_multi_tm() {
        let auto_port_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
        let tc_receiver = SyncTcCacher::default();
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
        let mut packet_id_lookup = HashSet::new();
        packet_id_lookup.insert(TEST_PACKET_ID_0);
        packet_id_lookup.insert(TEST_PACKET_ID_1);
        let mut tcp_server = generic_tmtc_server(
            &auto_port_addr,
            tc_receiver.clone(),
            tm_source,
            packet_id_lookup,
            None,
        );
        let dest_addr = tcp_server
            .local_addr()
            .expect("retrieving dest addr failed");
        let conn_handled: Arc<AtomicBool> = Default::default();
        let set_if_done = conn_handled.clone();

        // Call the connection handler in separate thread, does block.
        thread::spawn(move || {
            let result = tcp_server.handle_next_connection();
            if result.is_err() {
                panic!("handling connection failed: {:?}", result.unwrap_err());
            }
            let conn_result = result.unwrap();
            assert_eq!(
                conn_result.num_received_tcs, 2,
                "wrong number of received TCs"
            );
            assert_eq!(conn_result.num_sent_tms, 2, "wrong number of sent TMs");
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
        let mut tc_queue = tc_receiver.tc_queue.lock().unwrap();
        assert_eq!(tc_queue.len(), 2);
        assert_eq!(tc_queue.pop_front().unwrap(), tc_0);
        assert_eq!(tc_queue.pop_front().unwrap(), tc_1);
    }
}
