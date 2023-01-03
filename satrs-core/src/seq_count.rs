#[cfg(feature = "alloc")]
use dyn_clone::DynClone;
#[cfg(feature = "std")]
pub use stdmod::*;

pub trait SequenceCountProvider<Raw> {
    fn get(&self) -> Raw;
    fn increment(&mut self);
    fn get_and_increment(&mut self) -> Raw {
        let val = self.get();
        self.increment();
        val
    }
}

#[cfg(feature = "alloc")]
pub trait SequenceCountProviderClonable<Raw>: SequenceCountProvider<Raw> + DynClone {}
#[cfg(feature = "alloc")]
dyn_clone::clone_trait_object!(SequenceCountProviderClonable<u16>);

#[derive(Default, Clone)]
pub struct SimpleSeqCountProvider {
    seq_count: u16,
}

impl SequenceCountProvider<u16> for SimpleSeqCountProvider {
    fn get(&self) -> u16 {
        self.seq_count
    }

    fn increment(&mut self) {
        if self.seq_count == u16::MAX {
            self.seq_count = 0;
            return;
        }
        self.seq_count += 1;
    }
}

#[cfg(feature = "alloc")]
impl SequenceCountProviderClonable<u16> for SimpleSeqCountProvider {}

#[cfg(feature = "std")]
pub mod stdmod {
    use super::*;
    use std::sync::atomic::{AtomicU16, Ordering};
    use std::sync::Arc;

    #[derive(Clone, Default)]
    pub struct SyncSeqCountProvider {
        seq_count: Arc<AtomicU16>,
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

    impl SequenceCountProviderClonable<u16> for SyncSeqCountProvider {}
}
