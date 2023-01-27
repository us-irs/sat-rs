use core::cell::Cell;
use core::sync::atomic::{AtomicU16, Ordering};
#[cfg(feature = "alloc")]
use dyn_clone::DynClone;
#[cfg(feature = "std")]
pub use stdmod::*;

/// Core trait for objects which can provide a sequence count.
///
/// The core functions are not mutable on purpose to allow easier usage with
/// static structs when using the interior mutability pattern. This can be achieved by using
/// [Cell], [core::cell::RefCell] or atomic types.
pub trait SequenceCountProviderCore<Raw> {
    fn get(&self) -> Raw;

    fn increment(&self);

    // TODO: Maybe remove this?
    fn increment_mut(&mut self) {
        self.increment();
    }

    fn get_and_increment(&self) -> Raw {
        let val = self.get();
        self.increment();
        val
    }

    // TODO: Maybe remove this?
    fn get_and_increment_mut(&mut self) -> Raw {
        self.get_and_increment()
    }
}

/// Extension trait which allows cloning a sequence count provider after it was turned into
/// a trait object.
#[cfg(feature = "alloc")]
pub trait SequenceCountProvider<Raw>: SequenceCountProviderCore<Raw> + DynClone {}
#[cfg(feature = "alloc")]
dyn_clone::clone_trait_object!(SequenceCountProvider<u16>);
#[cfg(feature = "alloc")]
impl<T, Raw> SequenceCountProvider<Raw> for T where T: SequenceCountProviderCore<Raw> + Clone {}

#[derive(Default, Clone)]
pub struct SeqCountProviderSimple {
    seq_count: Cell<u16>,
}

impl SequenceCountProviderCore<u16> for SeqCountProviderSimple {
    fn get(&self) -> u16 {
        self.seq_count.get()
    }

    fn increment(&self) {
        self.get_and_increment();
    }

    fn get_and_increment(&self) -> u16 {
        let curr_count = self.seq_count.get();

        if curr_count == u16::MAX {
            self.seq_count.set(0);
        } else {
            self.seq_count.set(curr_count + 1);
        }
        curr_count
    }
}

pub struct SeqCountProviderAtomicRef {
    atomic: AtomicU16,
    ordering: Ordering,
}

impl SeqCountProviderAtomicRef {
    pub const fn new(ordering: Ordering) -> Self {
        Self {
            atomic: AtomicU16::new(0),
            ordering,
        }
    }
}

impl SequenceCountProviderCore<u16> for SeqCountProviderAtomicRef {
    fn get(&self) -> u16 {
        self.atomic.load(self.ordering)
    }

    fn increment(&self) {
        self.atomic.fetch_add(1, self.ordering);
    }

    fn get_and_increment(&self) -> u16 {
        self.atomic.fetch_add(1, self.ordering)
    }
}

#[cfg(feature = "std")]
pub mod stdmod {
    use super::*;
    use std::sync::Arc;

    #[derive(Clone, Default)]
    pub struct SeqCountProviderSyncClonable {
        seq_count: Arc<AtomicU16>,
    }

    impl SequenceCountProviderCore<u16> for SeqCountProviderSyncClonable {
        fn get(&self) -> u16 {
            self.seq_count.load(Ordering::SeqCst)
        }

        fn increment(&self) {
            self.seq_count.fetch_add(1, Ordering::SeqCst);
        }

        fn get_and_increment(&self) -> u16 {
            self.seq_count.fetch_add(1, Ordering::SeqCst)
        }
    }
}
