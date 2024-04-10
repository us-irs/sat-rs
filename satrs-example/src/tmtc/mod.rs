use log::warn;
use satrs::pus::{
    EcssTcAndToken, MpscTmAsVecSender, MpscTmInSharedPoolSenderBounded, ReceivesEcssPusTc,
};
use satrs::spacepackets::SpHeader;
use std::sync::mpsc::{self, Receiver, SendError, Sender, SyncSender, TryRecvError};
use thiserror::Error;

use crate::pus::PusReceiver;
use satrs::pool::{PoolProvider, SharedStaticMemoryPool, StoreAddr, StoreError};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::PusPacket;
use satrs::tmtc::ReceivesCcsdsTc;

pub mod ccsds;
pub mod tm_funnel;

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
pub struct SharedTcPool {
    pub pool: SharedStaticMemoryPool,
}

impl SharedTcPool {
    pub fn add_pus_tc(&mut self, pus_tc: &PusTcReader) -> Result<StoreAddr, StoreError> {
        let mut pg = self.pool.write().expect("error locking TC store");
        let addr = pg.free_element(pus_tc.len_packed(), |buf| {
            buf[0..pus_tc.len_packed()].copy_from_slice(pus_tc.raw_data());
        })?;
        Ok(addr)
    }
}

#[derive(Clone)]
pub struct PusTcSourceProviderSharedPool {
    pub tc_source: SyncSender<StoreAddr>,
    pub shared_pool: SharedTcPool,
}

impl PusTcSourceProviderSharedPool {
    #[allow(dead_code)]
    pub fn clone_backing_pool(&self) -> SharedStaticMemoryPool {
        self.shared_pool.pool.clone()
    }
}

impl ReceivesEcssPusTc for PusTcSourceProviderSharedPool {
    type Error = MpscStoreAndSendError;

    fn pass_pus_tc(&mut self, _: &SpHeader, pus_tc: &PusTcReader) -> Result<(), Self::Error> {
        let addr = self.shared_pool.add_pus_tc(pus_tc)?;
        self.tc_source.send(addr)?;
        Ok(())
    }
}

impl ReceivesCcsdsTc for PusTcSourceProviderSharedPool {
    type Error = MpscStoreAndSendError;

    fn pass_ccsds(&mut self, _: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
        let mut pool = self.shared_pool.pool.write().expect("locking pool failed");
        let addr = pool.add(tc_raw)?;
        drop(pool);
        self.tc_source.send(addr)?;
        Ok(())
    }
}

// Newtype, can not implement necessary traits on MPSC sender directly because of orphan rules.
#[derive(Clone)]
pub struct PusTcSourceProviderDynamic(pub Sender<Vec<u8>>);

impl ReceivesEcssPusTc for PusTcSourceProviderDynamic {
    type Error = SendError<Vec<u8>>;

    fn pass_pus_tc(&mut self, _: &SpHeader, pus_tc: &PusTcReader) -> Result<(), Self::Error> {
        self.0.send(pus_tc.raw_data().to_vec())?;
        Ok(())
    }
}

impl ReceivesCcsdsTc for PusTcSourceProviderDynamic {
    type Error = mpsc::SendError<Vec<u8>>;

    fn pass_ccsds(&mut self, _: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
        self.0.send(tc_raw.to_vec())?;
        Ok(())
    }
}

// TC source components where static pools are the backing memory of the received telecommands.
pub struct TcSourceTaskStatic {
    shared_tc_pool: SharedTcPool,
    tc_receiver: Receiver<StoreAddr>,
    tc_buf: [u8; 4096],
    pus_receiver: PusReceiver<MpscTmInSharedPoolSenderBounded>,
}

impl TcSourceTaskStatic {
    pub fn new(
        shared_tc_pool: SharedTcPool,
        tc_receiver: Receiver<StoreAddr>,
        pus_receiver: PusReceiver<MpscTmInSharedPoolSenderBounded>,
    ) -> Self {
        Self {
            shared_tc_pool,
            tc_receiver,
            tc_buf: [0; 4096],
            pus_receiver,
        }
    }

    pub fn periodic_operation(&mut self) {
        self.poll_tc();
    }

    pub fn poll_tc(&mut self) -> bool {
        match self.tc_receiver.try_recv() {
            Ok(addr) => {
                let pool = self
                    .shared_tc_pool
                    .pool
                    .read()
                    .expect("locking tc pool failed");
                pool.read(&addr, &mut self.tc_buf)
                    .expect("reading pool failed");
                drop(pool);
                match PusTcReader::new(&self.tc_buf) {
                    Ok((pus_tc, _)) => {
                        self.pus_receiver
                            .handle_tc_packet(
                                satrs::pus::TcInMemory::StoreAddr(addr),
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

// TC source components where the heap is the backing memory of the received telecommands.
pub struct TcSourceTaskDynamic {
    pub tc_receiver: Receiver<Vec<u8>>,
    pus_receiver: PusReceiver<MpscTmAsVecSender>,
}

impl TcSourceTaskDynamic {
    pub fn new(
        tc_receiver: Receiver<Vec<u8>>,
        pus_receiver: PusReceiver<MpscTmAsVecSender>,
    ) -> Self {
        Self {
            tc_receiver,
            pus_receiver,
        }
    }

    pub fn periodic_operation(&mut self) {
        self.poll_tc();
    }

    pub fn poll_tc(&mut self) -> bool {
        match self.tc_receiver.try_recv() {
            Ok(tc) => match PusTcReader::new(&tc) {
                Ok((pus_tc, _)) => {
                    self.pus_receiver
                        .handle_tc_packet(
                            satrs::pus::TcInMemory::Vec(tc.clone()),
                            pus_tc.service(),
                            &pus_tc,
                        )
                        .ok();
                    true
                }
                Err(e) => {
                    warn!("error creating PUS TC from raw data: {e}");
                    warn!("raw data: {:x?}", tc);
                    true
                }
            },
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
