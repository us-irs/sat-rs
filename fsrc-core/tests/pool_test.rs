use fsrc_core::pool::{LocalPool, PoolCfg, StoreAddr, StoreError};
use std::sync::{Arc, RwLock};
use std::thread;

struct PoolAccessDummy<'a> {
    pool: &'a mut LocalPool,
    addr: Option<StoreAddr>,
    no_deletion: bool,
}

impl PoolAccessDummy<'_> {
    #[allow(dead_code)]
    fn modify(&mut self, addr: &StoreAddr) -> Result<&mut [u8], StoreError> {
        self.pool.modify(addr)
    }

    #[allow(dead_code)]
    fn release(&mut self) {
        self.no_deletion = true;
    }
}

impl Drop for PoolAccessDummy<'_> {
    fn drop(&mut self) {
        if self.no_deletion {
            println!("Pool access: Drop with no deletion")
        } else {
            if let Some(addr) = self.addr {
                let res = self.pool.delete(addr);
                if res.is_err() {
                    println!("Pool access: Deletion failed");
                }
            }
            println!("Pool access: Drop with deletion");
        }
    }
}

fn main() {
    println!("Hello World");
    let pool_cfg = PoolCfg::new(vec![(16, 6), (32, 3), (8, 12)]);
    let shared_dummy = Arc::new(RwLock::new(LocalPool::new(pool_cfg)));
    let _shared_clone = shared_dummy.clone();
    let jh0 = thread::spawn(move || loop {
        {
            let mut dummy = shared_dummy.write().unwrap();
            let dummy_data = [0, 1, 2, 3];
            let _addr = dummy.add(&dummy_data).expect("Writing data failed");

            // let mut accessor = dummy.modify_with_accessor();
            // let buf = accessor.modify();
        }
    });

    let jh1 = thread::spawn(move || loop {
        {
            // let dummy = shared_clone.read().unwrap();
            // let buf = dummy.read();
            // println!("Buffer 0: {:?}", buf[0]);
        }

        //let mut dummy = shared_clone.write().unwrap();
        //let mut accessor = dummy.modify_with_accessor();
        //let buf = accessor.modify();
        //buf[0] = 3;
        //accessor.release();
    });
    jh0.join().unwrap();
    jh1.join().unwrap();
}
