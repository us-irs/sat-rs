use std::sync::{Arc, RwLock};
use std::thread;

struct PoolDummy {
    test_buf: [u8; 128],
}

struct PoolAccessDummy<'a> {
    pool_dummy: &'a mut PoolDummy,
    no_deletion: bool,
}

impl PoolAccessDummy<'_> {
    fn modify(&mut self) -> &mut [u8] {
        self.pool_dummy.modify()
    }

    fn release(&mut self) {
        self.no_deletion = true;
    }
}

impl Drop for PoolAccessDummy<'_> {
    fn drop(&mut self) {
        if self.no_deletion {
            println!("Pool access: Drop with no deletion")
        } else {
            self.pool_dummy.delete();
            println!("Pool access: Drop with deletion");
        }
    }
}

impl Default for PoolDummy {
    fn default() -> Self {
        PoolDummy { test_buf: [0; 128] }
    }
}

impl PoolDummy {
    fn modify(&mut self) -> &mut [u8] {
        self.test_buf.as_mut_slice()
    }

    fn modify_with_accessor(&mut self) -> PoolAccessDummy {
        PoolAccessDummy {
            pool_dummy: self,
            no_deletion: false,
        }
    }

    fn read(&self) -> &[u8] {
        self.test_buf.as_slice()
    }

    fn delete(&mut self) {
        println!("Store content was deleted");
    }
}
fn main() {
    println!("Hello World");
    let shared_dummy = Arc::new(RwLock::new(PoolDummy::default()));
    let shared_clone = shared_dummy.clone();
    let jh0 = thread::spawn(move || loop {
        {
            let mut dummy = shared_dummy.write().unwrap();
            let buf = dummy.modify();
            buf[0] = 1;

            let mut accessor = dummy.modify_with_accessor();
            let buf = accessor.modify();
            buf[0] = 2;
        }
    });

    let jh1 = thread::spawn(move || loop {
        {
            let dummy = shared_clone.read().unwrap();
            let buf = dummy.read();
            println!("Buffer 0: {:?}", buf[0]);
        }

        let mut dummy = shared_clone.write().unwrap();
        let mut accessor = dummy.modify_with_accessor();
        let buf = accessor.modify();
        buf[0] = 3;
        accessor.release();
    });
    jh0.join().unwrap();
    jh1.join().unwrap();
}
