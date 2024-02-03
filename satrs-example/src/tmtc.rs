use log::warn;
use satrs_core::pus::{EcssTcAndToken, ReceivesEcssPusTc};
use satrs_core::spacepackets::SpHeader;
use std::sync::mpsc::{Receiver, SendError, Sender, TryRecvError};
use thiserror::Error;

use crate::pus::PusReceiver;
use satrs_core::pool::{PoolProviderMemInPlace, SharedStaticMemoryPool, StoreAddr, StoreError};
use satrs_core::spacepackets::ecss::tc::PusTcReader;
use satrs_core::spacepackets::ecss::PusPacket;
use satrs_core::tmtc::tm_helper::SharedTmStore;
use satrs_core::tmtc::ReceivesCcsdsTc;

pub struct TmArgs {
    pub tm_store: SharedTmStore,
    pub tm_sink_sender: Sender<StoreAddr>,
    pub tm_udp_server_rx: Receiver<StoreAddr>,
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
    TcSend(#[from] SendError<EcssTcAndToken>),
    #[error("TMTC send error: {0}")]
    TmTcSend(#[from] SendError<StoreAddr>),
}

#[derive(Clone)]
pub struct TcStore {
    pub pool: SharedStaticMemoryPool,
}

impl TcStore {
    pub fn add_pus_tc(&mut self, pus_tc: &PusTcReader) -> Result<StoreAddr, StoreError> {
        let mut pg = self.pool.write().expect("error locking TC store");
        let (addr, buf) = pg.free_element(pus_tc.len_packed())?;
        buf[0..pus_tc.len_packed()].copy_from_slice(pus_tc.raw_data());
        Ok(addr)
    }
}

pub struct TmFunnel {
    pub tm_funnel_rx: Receiver<StoreAddr>,
    pub tm_server_tx: Sender<StoreAddr>,
}

#[derive(Clone)]
pub struct PusTcSource {
    pub tc_source: Sender<StoreAddr>,
    pub tc_store: TcStore,
}

impl ReceivesEcssPusTc for PusTcSource {
    type Error = MpscStoreAndSendError;

    fn pass_pus_tc(&mut self, _: &SpHeader, pus_tc: &PusTcReader) -> Result<(), Self::Error> {
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

pub struct TmtcTask {
    tc_args: TcArgs,
    tc_buf: [u8; 4096],
    pus_receiver: PusReceiver,
}

impl TmtcTask {
    pub fn new(tc_args: TcArgs, pus_receiver: PusReceiver) -> Self {
        Self {
            tc_args,
            tc_buf: [0; 4096],
            pus_receiver,
        }
    }

    pub fn periodic_operation(&mut self) {
        self.poll_tc();
    }

    pub fn poll_tc(&mut self) -> bool {
        match self.tc_args.tc_receiver.try_recv() {
            Ok(addr) => {
                let pool = self
                    .tc_args
                    .tc_source
                    .tc_store
                    .pool
                    .read()
                    .expect("locking tc pool failed");
                let data = pool.read(&addr).expect("reading pool failed");
                self.tc_buf[0..data.len()].copy_from_slice(data);
                drop(pool);
                match PusTcReader::new(&self.tc_buf) {
                    Ok((pus_tc, _)) => {
                        self.pus_receiver
                            .handle_tc_packet(
                                satrs_core::pus::TcInMemory::StoreAddr(addr),
                                pus_tc.service(),
                                &pus_tc,
                            )
                            .ok();
                        true
                    }
                    Err(e) => {
                        warn!("error creating PUS TC from raw data: {e}");
                        warn!("raw data: {:x?}", self.tc_buf);
                        true
                    }
                }
            }
            Err(e) => match e {
                TryRecvError::Empty => false,
                TryRecvError::Disconnected => {
                    warn!("tmtc thread: sender disconnected");
                    false
                }
            },
        }
    }
}
