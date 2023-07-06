use core::cell::Cell;
use core::sync::atomic::{AtomicU16, Ordering};
#[cfg(feature = "alloc")]
use dyn_clone::DynClone;
use paste::paste;
use spacepackets::MAX_SEQ_COUNT;
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

    fn get_and_increment(&self) -> Raw {
        let val = self.get();
        self.increment();
        val
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
pub struct SeqCountProviderSimple<T: Copy> {
    seq_count: Cell<T>,
    max_val: T,
}

macro_rules! impl_for_primitives {
    ($($ty: ident,)+) => {
        $(
            paste! {
                impl SeqCountProviderSimple<$ty> {
                    pub fn [<new_ $ty _max_val>](max_val: $ty) -> Self {
                        Self {
                            seq_count: Cell::new(0),
                            max_val,
                        }
                    }

                    pub fn [<new_ $ty>]() -> Self {
                        Self {
                            seq_count: Cell::new(0),
                            max_val: $ty::MAX
                        }
                    }
                }

                impl SequenceCountProviderCore<$ty> for SeqCountProviderSimple<$ty> {
                    fn get(&self) -> $ty {
                        self.seq_count.get()
                    }

                    fn increment(&self) {
                        self.get_and_increment();
                    }

                    fn get_and_increment(&self) -> $ty {
                        let curr_count = self.seq_count.get();

                        if curr_count == self.max_val {
                            self.seq_count.set(0);
                        } else {
                            self.seq_count.set(curr_count + 1);
                        }
                        curr_count
                    }
                }
            }
        )+
    }
}

impl_for_primitives!(u8, u16, u32, u64,);

/// This is a sequence count provider which wraps around at [MAX_SEQ_COUNT].
pub struct CcsdsSimpleSeqCountProvider {
    provider: SeqCountProviderSimple<u16>,
}

impl CcsdsSimpleSeqCountProvider {
    pub fn new() -> Self {
        Self {
            provider: SeqCountProviderSimple::new_u16_max_val(MAX_SEQ_COUNT),
        }
    }
}

impl Default for CcsdsSimpleSeqCountProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SequenceCountProviderCore<u16> for CcsdsSimpleSeqCountProvider {
    delegate::delegate! {
        to self.provider {
            fn get(&self) -> u16;
            fn increment(&self);
            fn get_and_increment(&self) -> u16;
        }
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

#[cfg(test)]
mod tests {
    use crate::seq_count::{
        CcsdsSimpleSeqCountProvider, SeqCountProviderSimple, SequenceCountProviderCore,
    };
    use spacepackets::MAX_SEQ_COUNT;

    #[test]
    fn test_u8_counter() {
        let u8_counter = SeqCountProviderSimple::new_u8();
        assert_eq!(u8_counter.get(), 0);
        assert_eq!(u8_counter.get_and_increment(), 0);
        assert_eq!(u8_counter.get_and_increment(), 1);
        assert_eq!(u8_counter.get(), 2);
    }

    #[test]
    fn test_u8_counter_overflow() {
        let u8_counter = SeqCountProviderSimple::new_u8();
        for _ in 0..256 {
            u8_counter.increment();
        }
        assert_eq!(u8_counter.get(), 0);
    }

    #[test]
    fn test_ccsds_counter() {
        let ccsds_counter = CcsdsSimpleSeqCountProvider::default();
        assert_eq!(ccsds_counter.get(), 0);
        assert_eq!(ccsds_counter.get_and_increment(), 0);
        assert_eq!(ccsds_counter.get_and_increment(), 1);
        assert_eq!(ccsds_counter.get(), 2);
    }

    #[test]
    fn test_ccsds_counter_overflow() {
        let ccsds_counter = CcsdsSimpleSeqCountProvider::default();
        for _ in 0..MAX_SEQ_COUNT + 1 {
            ccsds_counter.increment();
        }
        assert_eq!(ccsds_counter.get(), 0);
    }
}
