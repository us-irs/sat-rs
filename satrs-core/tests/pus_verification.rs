//#[cfg(feature = "crossbeam")]
pub mod crossbeam_test {
    use hashbrown::HashMap;
    use satrs_core::pool::{
        PoolProvider, PoolProviderWithGuards, StaticMemoryPool, StaticPoolConfig,
    };
    use satrs_core::pus::verification::{
        FailParams, RequestId, VerificationReporterCfg, VerificationReporterWithSender,
    };
    use satrs_core::pus::CrossbeamTmInStoreSender;
    use satrs_core::tmtc::tm_helper::SharedTmPool;
    use spacepackets::ecss::tc::{PusTcCreator, PusTcReader, PusTcSecondaryHeader};
    use spacepackets::ecss::tm::PusTmReader;
    use spacepackets::ecss::{EcssEnumU16, EcssEnumU8, PusPacket, WritablePusPacket};
    use spacepackets::SpHeader;
    use std::sync::{Arc, RwLock};
    use std::thread;
    use std::time::Duration;

    const TEST_APID: u16 = 0x03;
    const FIXED_STAMP: [u8; 7] = [0; 7];
    const PACKETS_SENT: u8 = 8;

    /// This test also shows how the verification report could be used in a multi-threaded context,
    /// wrapping it into an [Arc] and [Mutex] and then passing it to two threads.
    ///
    ///  - The first thread generates a acceptance, a start, two steps and one completion report
    ///  - The second generates an acceptance and start success report and a completion failure
    ///  - The third thread is the verification receiver. In the test case, it verifies the other two
    ///    threads have sent the correct expected verification reports
    #[test]
    fn test_shared_reporter() {
        // We use a synced sequence count provider here because both verification reporters have the
        // the same APID. If they had distinct APIDs, the more correct approach would be to have
        // each reporter have an own sequence count provider.
        let cfg = VerificationReporterCfg::new(TEST_APID, 1, 2, 8).unwrap();
        // Shared pool object to store the verification PUS telemetry
        let pool_cfg =
            StaticPoolConfig::new(vec![(10, 32), (10, 64), (10, 128), (10, 1024)], false);
        let shared_tm_pool = SharedTmPool::new(StaticMemoryPool::new(pool_cfg.clone()));
        let shared_tc_pool_0 = Arc::new(RwLock::new(StaticMemoryPool::new(pool_cfg)));
        let shared_tc_pool_1 = shared_tc_pool_0.clone();
        let (tx, rx) = crossbeam_channel::bounded(10);
        let sender =
            CrossbeamTmInStoreSender::new(0, "verif_sender", shared_tm_pool.clone(), tx.clone());
        let mut reporter_with_sender_0 =
            VerificationReporterWithSender::new(&cfg, Box::new(sender));
        let mut reporter_with_sender_1 = reporter_with_sender_0.clone();
        // For test purposes, we retrieve the request ID from the TCs and pass them to the receiver
        // tread.
        let req_id_0;
        let req_id_1;

        let (tx_tc_0, rx_tc_0) = crossbeam_channel::bounded(3);
        let (tx_tc_1, rx_tc_1) = crossbeam_channel::bounded(3);
        {
            let mut tc_guard = shared_tc_pool_0.write().unwrap();
            let mut sph = SpHeader::tc_unseg(TEST_APID, 0, 0).unwrap();
            let tc_header = PusTcSecondaryHeader::new_simple(17, 1);
            let pus_tc_0 = PusTcCreator::new_no_app_data(&mut sph, tc_header, true);
            req_id_0 = RequestId::new(&pus_tc_0);
            let addr = tc_guard
                .free_element(pus_tc_0.len_written(), |buf| {
                    pus_tc_0.write_to_bytes(buf).unwrap();
                })
                .unwrap();
            tx_tc_0.send(addr).unwrap();
            let mut sph = SpHeader::tc_unseg(TEST_APID, 1, 0).unwrap();
            let tc_header = PusTcSecondaryHeader::new_simple(5, 1);
            let pus_tc_1 = PusTcCreator::new_no_app_data(&mut sph, tc_header, true);
            req_id_1 = RequestId::new(&pus_tc_1);
            let addr = tc_guard
                .free_element(pus_tc_0.len_written(), |buf| {
                    pus_tc_1.write_to_bytes(buf).unwrap();
                })
                .unwrap();
            tx_tc_1.send(addr).unwrap();
        }
        let verif_sender_0 = thread::spawn(move || {
            let mut tc_buf: [u8; 1024] = [0; 1024];
            let tc_addr = rx_tc_0
                .recv_timeout(Duration::from_millis(20))
                .expect("Receive timeout");
            let tc_len;
            {
                let mut tc_guard = shared_tc_pool_0.write().unwrap();
                let pg = tc_guard.read_with_guard(tc_addr);
                tc_len = pg.read(&mut tc_buf).unwrap();
            }
            let (_tc, _) = PusTcReader::new(&tc_buf[0..tc_len]).unwrap();

            let token = reporter_with_sender_0.add_tc_with_req_id(req_id_0);
            let accepted_token = reporter_with_sender_0
                .acceptance_success(token, Some(&FIXED_STAMP))
                .expect("Acceptance success failed");

            // Do some start handling here
            let started_token = reporter_with_sender_0
                .start_success(accepted_token, Some(&FIXED_STAMP))
                .expect("Start success failed");
            // Do some step handling here
            reporter_with_sender_0
                .step_success(&started_token, Some(&FIXED_STAMP), EcssEnumU8::new(0))
                .expect("Start success failed");

            // Finish up
            reporter_with_sender_0
                .step_success(&started_token, Some(&FIXED_STAMP), EcssEnumU8::new(1))
                .expect("Start success failed");
            reporter_with_sender_0
                .completion_success(started_token, Some(&FIXED_STAMP))
                .expect("Completion success failed");
        });

        let verif_sender_1 = thread::spawn(move || {
            let mut tc_buf: [u8; 1024] = [0; 1024];
            let tc_addr = rx_tc_1
                .recv_timeout(Duration::from_millis(20))
                .expect("Receive timeout");
            let tc_len;
            {
                let mut tc_guard = shared_tc_pool_1.write().unwrap();
                let pg = tc_guard.read_with_guard(tc_addr);
                tc_len = pg.read(&mut tc_buf).unwrap();
            }
            let (tc, _) = PusTcReader::new(&tc_buf[0..tc_len]).unwrap();
            let token = reporter_with_sender_1.add_tc(&tc);
            let accepted_token = reporter_with_sender_1
                .acceptance_success(token, Some(&FIXED_STAMP))
                .expect("Acceptance success failed");
            let started_token = reporter_with_sender_1
                .start_success(accepted_token, Some(&FIXED_STAMP))
                .expect("Start success failed");
            let fail_code = EcssEnumU16::new(2);
            let params = FailParams::new(Some(&FIXED_STAMP), &fail_code, None);
            reporter_with_sender_1
                .completion_failure(started_token, params)
                .expect("Completion success failed");
        });

        let verif_receiver = thread::spawn(move || {
            let mut packet_counter = 0;
            let mut tm_buf: [u8; 1024] = [0; 1024];
            let mut verif_map = HashMap::new();
            while packet_counter < PACKETS_SENT {
                let verif_addr = rx
                    .recv_timeout(Duration::from_millis(50))
                    .expect("Packet reception timeout");
                let tm_len;
                let shared_tm_store = shared_tm_pool.clone_backing_pool();
                {
                    let mut rg = shared_tm_store.write().expect("Error locking shared pool");
                    let store_guard = rg.read_with_guard(verif_addr);
                    tm_len = store_guard
                        .read(&mut tm_buf)
                        .expect("Error reading TM slice");
                }
                let (pus_tm, _) =
                    PusTmReader::new(&tm_buf[0..tm_len], 7).expect("Error reading verification TM");
                let req_id =
                    RequestId::from_bytes(&pus_tm.source_data()[0..RequestId::SIZE_AS_BYTES])
                        .expect("reading request ID from PUS TM source data failed");
                if !verif_map.contains_key(&req_id) {
                    let content = vec![pus_tm.subservice()];
                    verif_map.insert(req_id, content);
                } else {
                    let content = verif_map.get_mut(&req_id).unwrap();
                    content.push(pus_tm.subservice())
                }
                packet_counter += 1;
            }
            for (req_id, content) in verif_map {
                if req_id == req_id_1 {
                    assert_eq!(content[0], 1);
                    assert_eq!(content[1], 3);
                    assert_eq!(content[2], 8);
                } else if req_id == req_id_0 {
                    assert_eq!(content[0], 1);
                    assert_eq!(content[1], 3);
                    assert_eq!(content[2], 5);
                    assert_eq!(content[3], 5);
                    assert_eq!(content[4], 7);
                } else {
                    panic!("Unexpected request ID {:?}", req_id);
                }
            }
        });
        verif_sender_0.join().expect("Joining thread 0 failed");
        verif_sender_1.join().expect("Joining thread 1 failed");
        verif_receiver.join().expect("Joining thread 2 failed");
    }
}
