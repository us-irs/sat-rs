use fsrc_core::pool::{LocalPool, PoolCfg, PoolGuard, StoreAddr};
use std::ops::DerefMut;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread;

const DUMMY_DATA: [u8; 4] = [0, 1, 2, 3];

#[test]
fn threaded_usage() {
    let pool_cfg = PoolCfg::new(vec![(16, 6), (32, 3), (8, 12)]);
    let shared_dummy = Arc::new(RwLock::new(LocalPool::new(pool_cfg)));
    let shared_clone = shared_dummy.clone();
    let (tx, rx): (Sender<StoreAddr>, Receiver<StoreAddr>) = mpsc::channel();
    let jh0 = thread::spawn(move || {
        let mut dummy = shared_dummy.write().unwrap();
        let addr = dummy.add(&DUMMY_DATA).expect("Writing data failed");
        tx.send(addr).expect("Sending store address failed");
    });

    let jh1 = thread::spawn(move || {
        let mut pool_access = shared_clone.write().unwrap();
        let addr;
        {
            addr = rx.recv().expect("Receiving store address failed");
            let pg = PoolGuard::new(pool_access.deref_mut(), addr);
            let read_res = pg.read().expect("Reading failed");
            assert_eq!(read_res, DUMMY_DATA);
        }
        assert!(!pool_access.has_element_at(&addr).expect("Invalid address"));
    });
    jh0.join().unwrap();
    jh1.join().unwrap();
}
