use std::{io::Write, net::TcpStream};

use alloc::boxed::Box;

use crate::{
    encoding::{ccsds::PacketIdLookup, parse_buffer_for_ccsds_space_packets},
    tmtc::{ReceivesTc, TmPacketSource},
};

use super::tcp_server::{ConnectionResult, TcpTcParser, TcpTmSender, TcpTmtcError};

/// Concrete [TcpTcParser] implementation for the [].
pub struct CcsdsTcParser {
    packet_id_lookup: Box<dyn PacketIdLookup>,
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

#[cfg(test)]
mod tests {}
