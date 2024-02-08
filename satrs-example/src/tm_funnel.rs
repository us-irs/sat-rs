use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, Sender},
};

use satrs_core::{
    pool::{PoolProviderMemInPlace, StoreAddr},
    seq_count::{CcsdsSimpleSeqCountProvider, SequenceCountProviderCore},
    spacepackets::{
        ecss::{tm::PusTmZeroCopyWriter, PusPacket},
        time::cds::MIN_CDS_FIELD_LEN,
        CcsdsPacket,
    },
    tmtc::tm_helper::SharedTmPool,
};

use crate::tcp::SyncTcpTmSource;

#[derive(Default)]
pub struct CcsdsSeqCounterMap {
    apid_seq_counter_map: HashMap<u16, CcsdsSimpleSeqCountProvider>,
}
impl CcsdsSeqCounterMap {
    pub fn get_and_increment(&mut self, apid: u16) -> u16 {
        self.apid_seq_counter_map
            .entry(apid)
            .or_default()
            .get_and_increment()
    }
}

pub struct TmFunnelCommon {
    seq_counter_map: CcsdsSeqCounterMap,
    msg_counter_map: HashMap<u8, u16>,
    sync_tm_tcp_source: SyncTcpTmSource,
}

impl TmFunnelCommon {
    pub fn new(sync_tm_tcp_source: SyncTcpTmSource) -> Self {
        Self {
            seq_counter_map: Default::default(),
            msg_counter_map: Default::default(),
            sync_tm_tcp_source,
        }
    }

    // Applies common packet processing operations for PUS TM packets. This includes setting
    // a sequence counter
    fn apply_packet_processing(&mut self, mut zero_copy_writer: PusTmZeroCopyWriter) {
        // zero_copy_writer.set_apid(PUS_APID);
        zero_copy_writer.set_seq_count(
            self.seq_counter_map
                .get_and_increment(zero_copy_writer.apid()),
        );
        let entry = self
            .msg_counter_map
            .entry(zero_copy_writer.service())
            .or_insert(0);
        zero_copy_writer.set_msg_count(*entry);
        if *entry == u16::MAX {
            *entry = 0;
        } else {
            *entry += 1;
        }

        // This operation has to come last!
        zero_copy_writer.finish();
    }
}

pub struct TmFunnelStatic {
    common: TmFunnelCommon,
    shared_tm_store: SharedTmPool,
    tm_funnel_rx: Receiver<StoreAddr>,
    tm_server_tx: Sender<StoreAddr>,
}

impl TmFunnelStatic {
    pub fn new(
        shared_tm_store: SharedTmPool,
        sync_tm_tcp_source: SyncTcpTmSource,
        tm_funnel_rx: Receiver<StoreAddr>,
        tm_server_tx: Sender<StoreAddr>,
    ) -> Self {
        Self {
            common: TmFunnelCommon::new(sync_tm_tcp_source),
            shared_tm_store,
            tm_funnel_rx,
            tm_server_tx,
        }
    }

    pub fn operation(&mut self) {
        if let Ok(addr) = self.tm_funnel_rx.recv() {
            // Read the TM, set sequence counter and message counter, and finally update
            // the CRC.
            let shared_pool = self.shared_tm_store.clone_backing_pool();
            let mut pool_guard = shared_pool.write().expect("Locking TM pool failed");
            let tm_raw = pool_guard
                .modify(&addr)
                .expect("Reading TM from pool failed");
            let zero_copy_writer = PusTmZeroCopyWriter::new(tm_raw, MIN_CDS_FIELD_LEN)
                .expect("Creating TM zero copy writer failed");
            self.common.apply_packet_processing(zero_copy_writer);
            self.tm_server_tx
                .send(addr)
                .expect("Sending TM to server failed");
            self.common.sync_tm_tcp_source.add_tm(tm_raw);
        }
    }
}

pub struct TmFunnelDynamic {
    common: TmFunnelCommon,
    tm_funnel_rx: Receiver<Vec<u8>>,
    tm_server_tx: Sender<Vec<u8>>,
}

impl TmFunnelDynamic {
    pub fn new(
        sync_tm_tcp_source: SyncTcpTmSource,
        tm_funnel_rx: Receiver<Vec<u8>>,
        tm_server_tx: Sender<Vec<u8>>,
    ) -> Self {
        Self {
            common: TmFunnelCommon::new(sync_tm_tcp_source),
            tm_funnel_rx,
            tm_server_tx,
        }
    }

    pub fn operation(&mut self) {
        if let Ok(mut tm) = self.tm_funnel_rx.recv() {
            // Read the TM, set sequence counter and message counter, and finally update
            // the CRC.
            let zero_copy_writer = PusTmZeroCopyWriter::new(&mut tm, MIN_CDS_FIELD_LEN)
                .expect("Creating TM zero copy writer failed");
            self.common.apply_packet_processing(zero_copy_writer);
            self.tm_server_tx
                .send(tm.clone())
                .expect("Sending TM to server failed");
            self.common.sync_tm_tcp_source.add_tm(&tm);
        }
    }
}
