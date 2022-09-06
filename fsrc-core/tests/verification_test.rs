use fsrc_core::pool::{LocalPool, PoolCfg};
use fsrc_core::pus::verification::{
    FailParams, RequestId, StdVerifSender, VerificationReporter, VerificationReporterCfg,
};
use spacepackets::ecss::{EcssEnumU16, EcssEnumU8, PusPacket};
use spacepackets::tc::{PusTc, PusTcSecondaryHeader};
use spacepackets::tm::PusTm;
use spacepackets::SpHeader;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;

const TEST_APID: u16 = 0x03;
const FIXED_STAMP: [u8; 7] = [0; 7];
const PACKETS_SENT: u8 = 8;

#[test]
fn test_shared_reporter() {
    let seq_counter_0 = Arc::new(AtomicU16::new(0));
    let seq_counter_1 = seq_counter_0.clone();
    let cfg = VerificationReporterCfg::new(TEST_APID, 1, 2, 8);
    // Shared pool object to store the verification PUS telemetry
    let pool_cfg = PoolCfg::new(vec![(10, 32), (10, 64), (10, 128), (10, 1024)]);
    let shared_pool = Arc::new(RwLock::new(LocalPool::new(pool_cfg)));
    let (tx, rx) = mpsc::channel();
    let mut sender_0 = StdVerifSender::new(shared_pool.clone(), tx.clone());
    let mut sender_1 = StdVerifSender::new(shared_pool.clone(), tx);
    let reporter_0 = Arc::new(Mutex::new(VerificationReporter::new(cfg)));
    let reporter_1 = reporter_0.clone();

    let verif_sender_0 = thread::spawn(move || {
        let mut sph =
            SpHeader::tc(TEST_APID, seq_counter_0.fetch_add(1, Ordering::SeqCst), 0).unwrap();
        let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
        let pus_tc = PusTc::new(&mut sph, tc_header, None, true);
        let mut mg = reporter_0.lock().unwrap();
        let token = mg.add_tc(&pus_tc);
        let accepted_token = mg
            .acceptance_success(token, &mut sender_0, &FIXED_STAMP)
            .expect("Acceptance success failed");
        let started_token = mg
            .start_success(accepted_token, &mut sender_0, &FIXED_STAMP)
            .expect("Start success failed");
        mg.step_success(
            &started_token,
            &mut sender_0,
            &FIXED_STAMP,
            EcssEnumU8::new(0),
        )
        .expect("Start success failed");
        mg.step_success(
            &started_token,
            &mut sender_0,
            &FIXED_STAMP,
            EcssEnumU8::new(1),
        )
        .expect("Start success failed");
        mg.completion_success(started_token, &mut sender_0, &FIXED_STAMP)
            .expect("Completion success failed");
    });

    let verif_sender_1 = thread::spawn(move || {
        let mut sph =
            SpHeader::tc(TEST_APID, seq_counter_1.fetch_add(1, Ordering::SeqCst), 0).unwrap();
        let tc_header = PusTcSecondaryHeader::new_simple(5, 1);
        let pus_tc = PusTc::new(&mut sph, tc_header, None, true);
        let mut mg = reporter_1.lock().unwrap();
        let token = mg.add_tc(&pus_tc);
        let accepted_token = mg
            .acceptance_success(token, &mut sender_1, &FIXED_STAMP)
            .expect("Acceptance success failed");
        let started_token = mg
            .start_success(accepted_token, &mut sender_1, &FIXED_STAMP)
            .expect("Start success failed");
        let fail_code = EcssEnumU16::new(2);
        let params = FailParams::new(&FIXED_STAMP, &fail_code, None);
        mg.completion_failure(started_token, &mut sender_1, params)
            .expect("Completion success failed");
    });

    let verif_receiver = thread::spawn(move || {
        let mut packet_counter = 0;
        let mut tm_buf: [u8; 1024] = [0; 1024];
        while packet_counter < PACKETS_SENT {
            let verif_addr = rx.recv().expect("Error receiving verification packet");
            let mut rg = shared_pool.write().expect("Error locking shared pool");
            let store_guard = rg.read_with_guard(verif_addr);
            let slice = store_guard.read().expect("Error reading TM slice");
            let tm_len = slice.len();
            tm_buf[0..tm_len].copy_from_slice(slice);
            drop(store_guard);
            drop(rg);
            let (pus_tm, _) = PusTm::new_from_raw_slice(&tm_buf[0..tm_len], 7)
                .expect("Error reading verification TM");
            let req_id = RequestId::from_bytes(
                &pus_tm.source_data().expect("Invalid TM source data")[0..RequestId::SIZE_AS_BYTES],
            )
            .unwrap();
            println!(
                "Received PUS Verification TM[{},{}] for request ID {:#08x}",
                pus_tm.service(),
                pus_tm.subservice(),
                req_id.raw()
            );
            packet_counter += 1;
        }
    });
    verif_sender_0.join().expect("Joining thread 0 failed");
    verif_sender_1.join().expect("Joining thread 1 failed");
    verif_receiver.join().expect("Joining thread 2 failed")
}
