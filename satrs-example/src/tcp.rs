use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use log::{info, warn};
use satrs::{
    hal::std::tcp_server::{HandledConnectionHandler, ServerConfig, TcpSpacepacketsServer},
    pus::ReceivesEcssPusTc,
    spacepackets::PacketId,
    tmtc::{CcsdsDistributor, CcsdsError, ReceivesCcsdsTc, TmPacketSourceCore},
};

use crate::ccsds::CcsdsReceiver;

#[derive(Default)]
pub struct ConnectionFinishedHandler {}

impl HandledConnectionHandler for ConnectionFinishedHandler {
    fn handled_connection(&mut self, info: satrs::hal::std::tcp_server::HandledConnectionInfo) {
        info!(
            "Served {} TMs and {} TCs for client {:?}",
            info.num_sent_tms, info.num_received_tcs, info.addr
        );
    }
}

#[derive(Default, Clone)]
pub struct SyncTcpTmSource {
    tm_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    max_packets_stored: usize,
    pub silent_packet_overwrite: bool,
}

impl SyncTcpTmSource {
    pub fn new(max_packets_stored: usize) -> Self {
        Self {
            tm_queue: Arc::default(),
            max_packets_stored,
            silent_packet_overwrite: true,
        }
    }

    pub fn add_tm(&mut self, tm: &[u8]) {
        let mut tm_queue = self.tm_queue.lock().expect("locking tm queue failec");
        if tm_queue.len() > self.max_packets_stored {
            if !self.silent_packet_overwrite {
                warn!("TPC TM source is full, deleting oldest packet");
            }
            tm_queue.pop_front();
        }
        tm_queue.push_back(tm.to_vec());
    }
}

impl TmPacketSourceCore for SyncTcpTmSource {
    type Error = ();

    fn retrieve_packet(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        let mut tm_queue = self.tm_queue.lock().expect("locking tm queue failed");
        if !tm_queue.is_empty() {
            let next_vec = tm_queue.front().unwrap();
            if buffer.len() < next_vec.len() {
                panic!(
                    "provided buffer too small, must be at least {} bytes",
                    next_vec.len()
                );
            }
            let next_vec = tm_queue.pop_front().unwrap();
            buffer[0..next_vec.len()].copy_from_slice(&next_vec);
            if next_vec.len() > 9 {
                let service = next_vec[7];
                let subservice = next_vec[8];
                info!("Sending PUS TM[{service},{subservice}]")
            } else {
                info!("Sending PUS TM");
            }
            return Ok(next_vec.len());
        }
        Ok(0)
    }
}

pub type TcpServerType<TcSource, MpscErrorType> = TcpSpacepacketsServer<
    SyncTcpTmSource,
    CcsdsDistributor<CcsdsReceiver<TcSource, MpscErrorType>, MpscErrorType>,
    HashSet<PacketId>,
    ConnectionFinishedHandler,
    (),
    CcsdsError<MpscErrorType>,
>;

pub struct TcpTask<
    TcSource: ReceivesCcsdsTc<Error = MpscErrorType>
        + ReceivesEcssPusTc<Error = MpscErrorType>
        + Clone
        + Send
        + 'static,
    MpscErrorType: 'static,
> {
    server: TcpServerType<TcSource, MpscErrorType>,
}

impl<
        TcSource: ReceivesCcsdsTc<Error = MpscErrorType>
            + ReceivesEcssPusTc<Error = MpscErrorType>
            + Clone
            + Send
            + 'static,
        MpscErrorType: 'static + core::fmt::Debug,
    > TcpTask<TcSource, MpscErrorType>
{
    pub fn new(
        cfg: ServerConfig,
        tm_source: SyncTcpTmSource,
        tc_receiver: CcsdsDistributor<CcsdsReceiver<TcSource, MpscErrorType>, MpscErrorType>,
        packet_id_lookup: HashSet<PacketId>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            server: TcpSpacepacketsServer::new(
                cfg,
                tm_source,
                tc_receiver,
                packet_id_lookup,
                ConnectionFinishedHandler::default(),
                None,
            )?,
        })
    }

    pub fn periodic_operation(&mut self) {
        loop {
            let result = self.server.handle_next_connection(None);
            match result {
                Ok(_conn_result) => (),
                Err(e) => {
                    warn!("TCP server error: {e:?}");
                }
            }
        }
    }
}
