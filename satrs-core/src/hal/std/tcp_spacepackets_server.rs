use delegate::delegate;
use std::{io::Write, net::{TcpStream, TcpListener, SocketAddr}};

use alloc::boxed::Box;

use crate::{
    encoding::{ccsds::PacketIdLookup, parse_buffer_for_ccsds_space_packets},
    tmtc::{ReceivesTc, TmPacketSource},
};

use super::tcp_server::{
    ConnectionResult, ServerConfig, TcpTcParser, TcpTmSender, TcpTmtcError, TcpTmtcGenericServer,
};

/// Concrete [TcpTcParser] implementation for the [].
pub struct CcsdsTcParser {
    packet_id_lookup: Box<dyn PacketIdLookup + Send>,
}

impl CcsdsTcParser {
    pub fn new(packet_id_lookup: Box<dyn PacketIdLookup + Send>) -> Self {
        Self { packet_id_lookup }
    }
}

impl<TmError, TcError: 'static> TcpTcParser<TmError, TcError> for CcsdsTcParser {
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
            self.packet_id_lookup.as_ref(),
            tc_receiver.upcast_mut(),
            next_write_idx,
        )
        .map_err(|e| TcpTmtcError::TcError(e))?;
        Ok(())
    }
}

/// Concrete [TcpTmSender] implementation for the [].
#[derive(Default)]
pub struct CcsdsTmSender {}

impl<TmError, TcError> TcpTmSender<TmError, TcError> for CcsdsTmSender {
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

/// TCP TMTC server implementation for exchange of tightly stuffed CCSDS space packets.
///
/// This serves only works if CCSDS space packets are the only packet type being exchanged.
/// It uses the CCSDS [spacepackets::PacketId] as the packet delimiter and start marker when
/// parsing for packets. The user specifies a set of expected [spacepackets::PacketId]s as part
/// of the server configuration for that purpose.
///
/// ## Example
///
/// The [TCP integration test](https://egit.irs.uni-stuttgart.de/rust/sat-rs/src/branch/main/satrs-core/tests/tcp_server_cobs.rs)
/// also serves as the example application for this module.
pub struct TcpSpacepacketsServer<TmError, TcError: 'static> {
    generic_server: TcpTmtcGenericServer<TmError, TcError, CcsdsTmSender, CcsdsTcParser>,
}

impl<TmError: 'static, TcError: 'static> TcpSpacepacketsServer<TmError, TcError> {
    /// Create a new TCP TMTC server which exchanges TMTC packets encoded with
    /// [COBS protocol](https://en.wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing).
    ///
    /// ## Parameter
    ///
    /// * `cfg` - Configuration of the server.
    /// * `tm_source` - Generic TM source used by the server to pull telemetry packets which are
    ///     then sent back to the client.
    /// * `tc_receiver` - Any received telecommands which were decoded successfully will be
    ///     forwarded to this TC receiver.
    pub fn new(
        cfg: ServerConfig,
        tm_source: Box<dyn TmPacketSource<Error = TmError>>,
        tc_receiver: Box<dyn ReceivesTc<Error = TcError>>,
        packet_id_lookup: Box<dyn PacketIdLookup + Send>
    ) -> Result<Self, TcpTmtcError<TmError, TcError>> {
        Ok(Self {
            generic_server: TcpTmtcGenericServer::new(
                cfg,
                CcsdsTcParser::new(packet_id_lookup),
                CcsdsTmSender::default(),
                tm_source,
                tc_receiver,
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
    use super::TcpSpacepacketsServer;

    #[test]
    fn test_basic() {
        let
        let server = TcpSpacepacketsServer::new(cfg, tm_source, tc_receiver, packet_id_lookup)
        
    }
}
