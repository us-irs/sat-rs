use std::{
    collections::HashMap,
    sync::mpsc::{self},
};

use arbitrary_int::{u11, u14};
use log::info;
use satrs::{
    spacepackets::{
        ecss::{tm::PusTmZeroCopyWriter, PusPacket},
        seq_count::{SequenceCounter, SequenceCounterCcsdsSimple},
        CcsdsPacket,
    },
    tmtc::PacketAsVec,
};
use satrs_example::CcsdsTmPacketOwned;

use crate::interface::tcp::SyncTcpTmSource;

#[derive(Default)]
pub struct CcsdsSeqCounterMap {
    apid_seq_counter_map: HashMap<u11, SequenceCounterCcsdsSimple>,
}
impl CcsdsSeqCounterMap {
    pub fn get_and_increment(&mut self, apid: u11) -> u14 {
        u14::new(
            self.apid_seq_counter_map
                .entry(apid)
                .or_default()
                .get_and_increment(),
        )
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
            .entry(zero_copy_writer.service_type_id())
            .or_insert(0);
        zero_copy_writer.set_msg_count(*entry);
        if *entry == u16::MAX {
            *entry = 0;
        } else {
            *entry += 1;
        }

        Self::packet_printout(&zero_copy_writer);
        // This operation has to come last!
        zero_copy_writer.finish();
    }

    fn packet_printout(tm: &PusTmZeroCopyWriter) {
        info!(
            "Sending PUS TM[{},{}] with APID {}",
            tm.service_type_id(),
            tm.message_subtype_id(),
            tm.apid()
        );
    }
}

/*
pub struct TmSinkStatic {
    common: TmFunnelCommon,
    shared_tm_store: SharedPacketPool,
    tm_funnel_rx: mpsc::Receiver<PacketInPool>,
    tm_server_tx: mpsc::SyncSender<PacketInPool>,
}

#[allow(dead_code)]
impl TmSinkStatic {
    pub fn new(
        shared_tm_store: SharedPacketPool,
        sync_tm_tcp_source: SyncTcpTmSource,
        tm_funnel_rx: mpsc::Receiver<PacketInPool>,
        tm_server_tx: mpsc::SyncSender<PacketInPool>,
    ) -> Self {
        Self {
            common: TmFunnelCommon::new(sync_tm_tcp_source),
            shared_tm_store,
            tm_funnel_rx,
            tm_server_tx,
        }
    }

    pub fn operation(&mut self) {
        if let Ok(pus_tm_in_pool) = self.tm_funnel_rx.recv() {
            // Read the TM, set sequence counter and message counter, and finally update
            // the CRC.
            let shared_pool = self.shared_tm_store.0.clone();
            let mut pool_guard = shared_pool.write().expect("Locking TM pool failed");
            let mut tm_copy = Vec::new();
            pool_guard
                .modify(&pus_tm_in_pool.store_addr, |buf| {
                    let zero_copy_writer = PusTmZeroCopyWriter::new(buf, MIN_CDS_FIELD_LEN, true)
                        .expect("Creating TM zero copy writer failed");
                    self.common.apply_packet_processing(zero_copy_writer);
                    tm_copy = buf.to_vec()
                })
                .expect("Reading TM from pool failed");
            self.tm_server_tx
                .send(pus_tm_in_pool)
                .expect("Sending TM to server failed");
            // We could also do this step in the update closure, but I'd rather avoid this, could
            // lead to nested locking.
            self.common.sync_tm_tcp_source.add_tm(&tm_copy);
        }
    }
}
*/

pub struct TmSink {
    common: TmFunnelCommon,
    tm_funnel_rx: mpsc::Receiver<CcsdsTmPacketOwned>,
    tm_server_tx: mpsc::SyncSender<CcsdsTmPacketOwned>,
}

//#[allow(dead_code)]
impl TmSink {
    pub fn new(
        sync_tm_tcp_source: SyncTcpTmSource,
        tm_funnel_rx: mpsc::Receiver<CcsdsTmPacketOwned>,
        tm_server_tx: mpsc::SyncSender<CcsdsTmPacketOwned>,
    ) -> Self {
        Self {
            common: TmFunnelCommon::new(sync_tm_tcp_source),
            tm_funnel_rx,
            tm_server_tx,
        }
    }

    pub fn operation(&mut self) {
        if let Ok(mut tm) = self.tm_funnel_rx.recv() {
            //let tm_raw = tm.to_vec();
            tm.sp_header.set_seq_count(
                self.common
                    .seq_counter_map
                    .get_and_increment(tm.sp_header.apid()),
            );
            // Read the TM, set sequence counter and message counter, and finally update
            // the CRC.
            /*
            let zero_copy_writer =
                PusTmZeroCopyWriter::new(&mut tm_raw, MIN_CDS_FIELD_LEN, true)
                    .expect("Creating TM zero copy writer failed");
            self.common.apply_packet_processing(zero_copy_writer);
            */
            self.common.sync_tm_tcp_source.add_tm(&tm.to_vec());
            self.tm_server_tx
                .send(tm)
                .expect("Sending TM to server failed");
        }
    }
}

//#[allow(dead_code)]
//pub enum TmSink {
//Static(TmSinkStatic),
//Heap(TmSinkDynamic),
//}

/*
impl TmSink {
    pub fn operation(&mut self) {
        match self {
            TmSink::Static(static_sink) => static_sink.operation(),
            TmSink::Heap(dynamic_sink) => dynamic_sink.operation(),
        }
    }
}
*/
