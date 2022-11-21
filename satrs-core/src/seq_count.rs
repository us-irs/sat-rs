use dyn_clone::DynClone;
#[cfg(feature = "std")]
pub use stdmod::*;

pub trait SequenceCountProvider<Raw>: DynClone {
    fn get(&self) -> Raw;
    fn increment(&mut self);
    fn get_and_increment(&mut self) -> Raw {
        let val = self.get();
        self.increment();
        val
    }
}

#[derive(Default, Clone)]
pub struct SimpleSeqCountProvider {
    seq_count: u16
}

dyn_clone::clone_trait_object!(SequenceCountProvider<u16>);

impl SequenceCountProvider<u16> for SimpleSeqCountProvider {
    fn get(&self) -> u16 {
        self.seq_count
    }

    fn increment(&mut self) {
        self.seq_count += 1;
    }
}

#[cfg(feature = "std")]
pub mod stdmod {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU16, Ordering};

    #[derive(Clone)]
    pub struct SyncSeqCountProvider {
        seq_count: Arc<AtomicU16>
    }

    impl SequenceCountProvider<u16> for SyncSeqCountProvider {
        fn get(&self) -> u16 {
            self.seq_count.load(Ordering::SeqCst)
        }

        fn increment(&mut self) {
            self.seq_count.fetch_add(1, Ordering::SeqCst);
        }

        fn get_and_increment(&mut self) -> u16 {
            self.seq_count.fetch_add(1, Ordering::SeqCst)
        }
    }
}
