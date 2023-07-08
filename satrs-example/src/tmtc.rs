use log::{info, warn};
use satrs_core::hal::host::udp_server::{ReceiveResult, UdpTcServer};
use std::net::SocketAddr;
use std::sync::mpsc::{Receiver, SendError, Sender, TryRecvError};
use std::thread;
use std::time::Duration;
use thiserror::Error;

use crate::ccsds::CcsdsReceiver;
use crate::pus::{PusReceiver, PusTcMpscRouter};
use satrs_core::pool::{SharedPool, StoreAddr, StoreError};
use satrs_core::pus::verification::StdVerifReporterWithSender;
use satrs_core::pus::AcceptedTc;
use satrs_core::spacepackets::ecss::{PusPacket, SerializablePusPacket};
use satrs_core::spacepackets::tc::PusTc;
use satrs_core::spacepackets::SpHeader;
use satrs_core::tmtc::tm_helper::SharedTmStore;
use satrs_core::tmtc::{CcsdsDistributor, CcsdsError, ReceivesCcsdsTc, ReceivesEcssPusTc};

pub const PUS_APID: u16 = 0x02;

pub struct TmArgs {
    pub tm_store: SharedTmStore,
    pub tm_sink_sender: Sender<StoreAddr>,
    pub tm_server_rx: Receiver<StoreAddr>,
}

pub struct TcArgs {
    pub tc_source: PusTcSource,
    pub tc_receiver: Receiver<StoreAddr>,
}

impl TcArgs {
    #[allow(dead_code)]
    fn split(self) -> (PusTcSource, Receiver<StoreAddr>) {
        (self.tc_source, self.tc_receiver)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MpscStoreAndSendError {
    #[error("Store error: {0}")]
    Store(#[from] StoreError),
    #[error("TC send error: {0}")]
    TcSend(#[from] SendError<AcceptedTc>),
    #[error("TMTC send error: {0}")]
    TmTcSend(#[from] SendError<StoreAddr>),
}

#[derive(Clone)]
pub struct TcStore {
    pub pool: SharedPool,
}

impl TcStore {
    pub fn add_pus_tc(&mut self, pus_tc: &PusTc) -> Result<StoreAddr, StoreError> {
        let mut pg = self.pool.write().expect("error locking TC store");
        let (addr, buf) = pg.free_element(pus_tc.len_packed())?;
        pus_tc
            .write_to_bytes(buf)
            .expect("writing PUS TC to store failed");
        Ok(addr)
    }
}

pub struct TmFunnel {
    pub tm_funnel_rx: Receiver<StoreAddr>,
    pub tm_server_tx: Sender<StoreAddr>,
}

pub struct UdpTmtcServer {
    udp_tc_server: UdpTcServer<CcsdsError<MpscStoreAndSendError>>,
    tm_rx: Receiver<StoreAddr>,
    tm_store: SharedPool,
}

#[derive(Clone)]
pub struct PusTcSource {
    pub tc_source: Sender<StoreAddr>,
    pub tc_store: TcStore,
}

impl ReceivesEcssPusTc for PusTcSource {
    type Error = MpscStoreAndSendError;

    fn pass_pus_tc(&mut self, _: &SpHeader, pus_tc: &PusTc) -> Result<(), Self::Error> {
        let addr = self.tc_store.add_pus_tc(pus_tc)?;
        self.tc_source.send(addr)?;
        Ok(())
    }
}

impl ReceivesCcsdsTc for PusTcSource {
    type Error = MpscStoreAndSendError;

    fn pass_ccsds(&mut self, _: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
        let mut pool = self.tc_store.pool.write().expect("locking pool failed");
        let addr = pool.add(tc_raw)?;
        drop(pool);
        self.tc_source.send(addr)?;
        Ok(())
    }
}

pub fn core_tmtc_task(
    socket_addr: SocketAddr,
    mut tc_args: TcArgs,
    tm_args: TmArgs,
    verif_reporter: StdVerifReporterWithSender,
    pus_router: PusTcMpscRouter,
) {
    let mut pus_receiver = PusReceiver::new(verif_reporter, pus_router);

    let ccsds_receiver = CcsdsReceiver {
        tc_source: tc_args.tc_source.clone(),
    };

    let ccsds_distributor = CcsdsDistributor::new(Box::new(ccsds_receiver));

    let udp_tc_server = UdpTcServer::new(socket_addr, 2048, Box::new(ccsds_distributor))
        .expect("creating UDP TMTC server failed");

    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_rx: tm_args.tm_server_rx,
        tm_store: tm_args.tm_store.backing_pool(),
    };

    let mut tc_buf: [u8; 4096] = [0; 4096];
    loop {
        core_tmtc_loop(
            &mut udp_tmtc_server,
            &mut tc_args,
            &mut tc_buf,
            &mut pus_receiver,
        );
        thread::sleep(Duration::from_millis(400));
    }
}

fn core_tmtc_loop(
    udp_tmtc_server: &mut UdpTmtcServer,
    tc_args: &mut TcArgs,
    tc_buf: &mut [u8],
    pus_receiver: &mut PusReceiver,
) {
    while poll_tc_server(udp_tmtc_server) {}
    match tc_args.tc_receiver.try_recv() {
        Ok(addr) => {
            let pool = tc_args
                .tc_source
                .tc_store
                .pool
                .read()
                .expect("locking tc pool failed");
            let data = pool.read(&addr).expect("reading pool failed");
            tc_buf[0..data.len()].copy_from_slice(data);
            drop(pool);
            match PusTc::from_bytes(tc_buf) {
                Ok((pus_tc, _)) => {
                    pus_receiver
                        .handle_tc_packet(addr, pus_tc.service(), &pus_tc)
                        .ok();
                }
                Err(e) => {
                    warn!("error creating PUS TC from raw data: {e}");
                    warn!("raw data: {tc_buf:x?}");
                }
            }
        }
        Err(e) => {
            if let TryRecvError::Disconnected = e {
                warn!("tmtc thread: sender disconnected")
            }
        }
    }
    if let Some(recv_addr) = udp_tmtc_server.udp_tc_server.last_sender() {
        core_tm_handling(udp_tmtc_server, &recv_addr);
    }
}

fn poll_tc_server(udp_tmtc_server: &mut UdpTmtcServer) -> bool {
    match udp_tmtc_server.udp_tc_server.try_recv_tc() {
        Ok(_) => true,
        Err(e) => match e {
            ReceiveResult::ReceiverError(e) => match e {
                CcsdsError::ByteConversionError(e) => {
                    warn!("packet error: {e:?}");
                    true
                }
                CcsdsError::CustomError(e) => {
                    warn!("mpsc store and send error {e:?}");
                    true
                }
            },
            ReceiveResult::IoError(e) => {
                warn!("IO error {e}");
                false
            }
            ReceiveResult::NothingReceived => false,
        },
    }
}

fn core_tm_handling(udp_tmtc_server: &mut UdpTmtcServer, recv_addr: &SocketAddr) {
    while let Ok(addr) = udp_tmtc_server.tm_rx.try_recv() {
        let store_lock = udp_tmtc_server.tm_store.write();
        if store_lock.is_err() {
            warn!("Locking TM store failed");
            continue;
        }
        let mut store_lock = store_lock.unwrap();
        let pg = store_lock.read_with_guard(addr);
        let read_res = pg.read();
        if read_res.is_err() {
            warn!("Error reading TM pool data");
            continue;
        }
        let buf = read_res.unwrap();
        if buf.len() > 9 {
            let service = buf[7];
            let subservice = buf[8];
            info!("Sending PUS TM[{service},{subservice}]")
        } else {
            info!("Sending PUS TM");
        }
        let result = udp_tmtc_server.udp_tc_server.socket.send_to(buf, recv_addr);
        if let Err(e) = result {
            warn!("Sending TM with UDP socket failed: {e}")
        }
    }
}
