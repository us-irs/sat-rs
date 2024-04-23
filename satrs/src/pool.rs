//! # Pool implementation providing memory pools for packet storage.
//!
//! This module provides generic abstractions for memory pools which provide a storage
//! machanism for variable sized data like Telemetry and Telecommand (TMTC) packets. The core
//! abstraction for this is the [PoolProvider] trait.
//!
//! It also contains the [StaticMemoryPool] as a concrete implementation which can be used to avoid
//! dynamic run-time allocations for the storage of TMTC packets.
//!
//! # Example for the [StaticMemoryPool]
//!
//! ```
//! use satrs::pool::{PoolProvider, StaticMemoryPool, StaticPoolConfig};
//!
//! // 4 buckets of 4 bytes, 2 of 8 bytes and 1 of 16 bytes
//! let pool_cfg = StaticPoolConfig::new(vec![(4, 4), (2, 8), (1, 16)], false);
//! let mut local_pool = StaticMemoryPool::new(pool_cfg);
//! let mut read_buf: [u8; 16] = [0; 16];
//! let mut addr;
//! {
//!     // Add new data to the pool
//!     let mut example_data = [0; 4];
//!     example_data[0] = 42;
//!     let res = local_pool.add(&example_data);
//!     assert!(res.is_ok());
//!     addr = res.unwrap();
//! }
//!
//! {
//!     // Read the store data back
//!     let res = local_pool.read(&addr, &mut read_buf);
//!     assert!(res.is_ok());
//!     let read_bytes = res.unwrap();
//!     assert_eq!(read_bytes, 4);
//!     assert_eq!(read_buf[0], 42);
//!     // Modify the stored data
//!     let res = local_pool.modify(&addr, |buf| {
//!         buf[0] = 12;
//!     });
//!     assert!(res.is_ok());
//! }
//!
//! {
//!     // Read the modified data back
//!     let res = local_pool.read(&addr, &mut read_buf);
//!     assert!(res.is_ok());
//!     let read_bytes = res.unwrap();
//!     assert_eq!(read_bytes, 4);
//!     assert_eq!(read_buf[0], 12);
//! }
//!
//! // Delete the stored data
//! local_pool.delete(addr);
//!
//! // Get a free element in the pool with an appropriate size
//! {
//!     let res = local_pool.free_element(12, |buf| {
//!         buf[0] = 7;
//!     });
//!     assert!(res.is_ok());
//!     addr = res.unwrap();
//! }
//!
//! // Read back the data
//! {
//!     // Read the store data back
//!     let res = local_pool.read(&addr, &mut read_buf);
//!     assert!(res.is_ok());
//!     let read_bytes = res.unwrap();
//!     assert_eq!(read_bytes, 12);
//!     assert_eq!(read_buf[0], 7);
//! }
//! ```
#[cfg(feature = "alloc")]
pub use alloc_mod::*;
use core::fmt::{Display, Formatter};
use delegate::delegate;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use spacepackets::ByteConversionError;
#[cfg(feature = "std")]
use std::error::Error;

type NumBlocks = u16;
pub type PoolAddr = u64;

/// Simple address type used for transactions with the local pool.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct StaticPoolAddr {
    pub(crate) pool_idx: u16,
    pub(crate) packet_idx: NumBlocks,
}

impl StaticPoolAddr {
    pub const INVALID_ADDR: u32 = 0xFFFFFFFF;

    pub fn raw(&self) -> u32 {
        ((self.pool_idx as u32) << 16) | self.packet_idx as u32
    }
}

impl From<StaticPoolAddr> for PoolAddr {
    fn from(value: StaticPoolAddr) -> Self {
        ((value.pool_idx as u64) << 16) | value.packet_idx as u64
    }
}

impl From<PoolAddr> for StaticPoolAddr {
    fn from(value: PoolAddr) -> Self {
        Self {
            pool_idx: ((value >> 16) & 0xff) as u16,
            packet_idx: (value & 0xff) as u16,
        }
    }
}

impl Display for StaticPoolAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "StoreAddr(pool index: {}, packet index: {})",
            self.pool_idx, self.packet_idx
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum StoreIdError {
    InvalidSubpool(u16),
    InvalidPacketIdx(u16),
}

impl Display for StoreIdError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            StoreIdError::InvalidSubpool(pool) => {
                write!(f, "invalid subpool, index: {pool}")
            }
            StoreIdError::InvalidPacketIdx(packet_idx) => {
                write!(f, "invalid packet index: {packet_idx}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl Error for StoreIdError {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PoolError {
    /// Requested data block is too large
    DataTooLarge(usize),
    /// The store is full. Contains the index of the full subpool
    StoreFull(u16),
    /// Store ID is invalid. This also includes partial errors where only the subpool is invalid
    InvalidStoreId(StoreIdError, Option<PoolAddr>),
    /// Valid subpool and packet index, but no data is stored at the given address
    DataDoesNotExist(PoolAddr),
    ByteConversionError(spacepackets::ByteConversionError),
    LockError,
    /// Internal or configuration errors
    InternalError(u32),
}

impl Display for PoolError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            PoolError::DataTooLarge(size) => {
                write!(f, "data to store with size {size} is too large")
            }
            PoolError::StoreFull(u16) => {
                write!(f, "store is too full. index for full subpool: {u16}")
            }
            PoolError::InvalidStoreId(id_e, addr) => {
                write!(f, "invalid store ID: {id_e}, address: {addr:?}")
            }
            PoolError::DataDoesNotExist(addr) => {
                write!(f, "no data exists at address {addr:?}")
            }
            PoolError::InternalError(e) => {
                write!(f, "internal error: {e}")
            }
            PoolError::ByteConversionError(e) => {
                write!(f, "store error: {e}")
            }
            PoolError::LockError => {
                write!(f, "lock error")
            }
        }
    }
}

impl From<ByteConversionError> for PoolError {
    fn from(value: ByteConversionError) -> Self {
        Self::ByteConversionError(value)
    }
}

#[cfg(feature = "std")]
impl Error for PoolError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let PoolError::InvalidStoreId(e, _) = self {
            return Some(e);
        }
        None
    }
}

/// Generic trait for pool providers which provide memory pools for variable sized data.
///
/// It specifies a basic API to [Self::add], [Self::modify], [Self::read] and [Self::delete] data
/// in the store at its core. The API was designed so internal optimizations can be performed
/// more easily and that is is also possible to make the pool structure [Sync] without the whole
/// pool structure being wrapped inside a lock.
pub trait PoolProvider {
    /// Add new data to the pool. The provider should attempt to reserve a memory block with the
    /// appropriate size and then copy the given data to the block. Yields a [PoolAddr] which can
    /// be used to access the data stored in the pool
    fn add(&mut self, data: &[u8]) -> Result<PoolAddr, PoolError>;

    /// The provider should attempt to reserve a free memory block with the appropriate size first.
    /// It then executes a user-provided closure and passes a mutable reference to that memory
    /// block to the closure. This allows the user to write data to the memory block.
    /// The function should yield a [PoolAddr] which can be used to access the data stored in the
    /// pool.
    fn free_element<W: FnMut(&mut [u8])>(
        &mut self,
        len: usize,
        writer: W,
    ) -> Result<PoolAddr, PoolError>;

    /// Modify data added previously using a given [PoolAddr]. The provider should use the store
    /// address to determine if a memory block exists for that address. If it does, it should
    /// call the user-provided closure and pass a mutable reference to the memory block
    /// to the closure. This allows the user to modify the memory block.
    fn modify<U: FnMut(&mut [u8])>(&mut self, addr: &PoolAddr, updater: U)
        -> Result<(), PoolError>;

    /// The provider should copy the data from the memory block to the user-provided buffer if
    /// it exists.
    fn read(&self, addr: &PoolAddr, buf: &mut [u8]) -> Result<usize, PoolError>;

    /// Delete data inside the pool given a [PoolAddr].
    fn delete(&mut self, addr: PoolAddr) -> Result<(), PoolError>;
    fn has_element_at(&self, addr: &PoolAddr) -> Result<bool, PoolError>;

    /// Retrieve the length of the data at the given store address.
    fn len_of_data(&self, addr: &PoolAddr) -> Result<usize, PoolError>;

    #[cfg(feature = "alloc")]
    fn read_as_vec(&self, addr: &PoolAddr) -> Result<alloc::vec::Vec<u8>, PoolError> {
        let mut vec = alloc::vec![0; self.len_of_data(addr)?];
        self.read(addr, &mut vec)?;
        Ok(vec)
    }
}

/// Extension trait which adds guarded pool access classes.
pub trait PoolProviderWithGuards: PoolProvider {
    /// This function behaves like [PoolProvider::read], but consumes the provided address
    /// and returns a RAII conformant guard object.
    ///
    /// Unless the guard [PoolRwGuard::release] method is called, the data for the
    /// given address will be deleted automatically when the guard is dropped.
    /// This can prevent memory leaks. Users can read the data and release the guard
    /// if the data in the store is valid for further processing. If the data is faulty, no
    /// manual deletion is necessary when returning from a processing function prematurely.
    fn read_with_guard(&mut self, addr: PoolAddr) -> PoolGuard<Self>;

    /// This function behaves like [PoolProvider::modify], but consumes the provided
    /// address and returns a RAII conformant guard object.
    ///
    /// Unless the guard [PoolRwGuard::release] method is called, the data for the
    /// given address will be deleted automatically when the guard is dropped.
    /// This can prevent memory leaks. Users can read (and modify) the data and release the guard
    /// if the data in the store is valid for further processing. If the data is faulty, no
    /// manual deletion is necessary when returning from a processing function prematurely.
    fn modify_with_guard(&mut self, addr: PoolAddr) -> PoolRwGuard<Self>;
}

pub struct PoolGuard<'a, MemProvider: PoolProvider + ?Sized> {
    pool: &'a mut MemProvider,
    pub addr: PoolAddr,
    no_deletion: bool,
    deletion_failed_error: Option<PoolError>,
}

/// This helper object can be used to safely access pool data without worrying about memory
/// leaks.
impl<'a, MemProvider: PoolProvider> PoolGuard<'a, MemProvider> {
    pub fn new(pool: &'a mut MemProvider, addr: PoolAddr) -> Self {
        Self {
            pool,
            addr,
            no_deletion: false,
            deletion_failed_error: None,
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize, PoolError> {
        self.pool.read(&self.addr, buf)
    }

    #[cfg(feature = "alloc")]
    pub fn read_as_vec(&self) -> Result<alloc::vec::Vec<u8>, PoolError> {
        self.pool.read_as_vec(&self.addr)
    }

    /// Releasing the pool guard will disable the automatic deletion of the data when the guard
    /// is dropped.
    pub fn release(&mut self) {
        self.no_deletion = true;
    }
}

impl<MemProvider: PoolProvider + ?Sized> Drop for PoolGuard<'_, MemProvider> {
    fn drop(&mut self) {
        if !self.no_deletion {
            if let Err(e) = self.pool.delete(self.addr) {
                self.deletion_failed_error = Some(e);
            }
        }
    }
}

pub struct PoolRwGuard<'a, MemProvider: PoolProvider + ?Sized> {
    guard: PoolGuard<'a, MemProvider>,
}

impl<'a, MemProvider: PoolProvider> PoolRwGuard<'a, MemProvider> {
    pub fn new(pool: &'a mut MemProvider, addr: PoolAddr) -> Self {
        Self {
            guard: PoolGuard::new(pool, addr),
        }
    }

    pub fn update<U: FnMut(&mut [u8])>(&mut self, updater: &mut U) -> Result<(), PoolError> {
        self.guard.pool.modify(&self.guard.addr, updater)
    }

    delegate!(
        to self.guard {
            pub fn read(&self, buf: &mut [u8]) -> Result<usize, PoolError>;
            /// Releasing the pool guard will disable the automatic deletion of the data when the guard
            /// is dropped.
            pub fn release(&mut self);
        }
    );
}

#[cfg(feature = "alloc")]
mod alloc_mod {
    use super::{PoolGuard, PoolProvider, PoolProviderWithGuards, PoolRwGuard, StaticPoolAddr};
    use crate::pool::{NumBlocks, PoolAddr, PoolError, StoreIdError};
    use alloc::vec;
    use alloc::vec::Vec;
    use spacepackets::ByteConversionError;
    #[cfg(feature = "std")]
    use std::sync::{Arc, RwLock};

    #[cfg(feature = "std")]
    pub type SharedStaticMemoryPool = Arc<RwLock<StaticMemoryPool>>;

    type PoolSize = usize;
    const STORE_FREE: PoolSize = PoolSize::MAX;
    pub const POOL_MAX_SIZE: PoolSize = STORE_FREE - 1;

    /// Configuration structure of the [static memory pool][StaticMemoryPool]
    ///
    /// # Parameters
    ///
    /// * `cfg` - Vector of tuples which represent a subpool. The first entry in the tuple specifies
    ///     the number of memory blocks in the subpool, the second entry the size of the blocks
    /// * `spill_to_higher_subpools` - Specifies whether data will be spilled to higher subpools
    ///     if the next fitting subpool is full. This is useful to ensure the pool remains useful
    ///     for all data sizes as long as possible. However, an undesirable side-effect might be
    ///     the chocking of larger subpools by underdimensioned smaller subpools.
    #[derive(Clone)]
    pub struct StaticPoolConfig {
        cfg: Vec<(NumBlocks, usize)>,
        spill_to_higher_subpools: bool,
    }

    impl StaticPoolConfig {
        pub fn new(cfg: Vec<(NumBlocks, usize)>, spill_to_higher_subpools: bool) -> Self {
            StaticPoolConfig {
                cfg,
                spill_to_higher_subpools,
            }
        }

        pub fn cfg(&self) -> &Vec<(NumBlocks, usize)> {
            &self.cfg
        }

        pub fn sanitize(&mut self) -> usize {
            self.cfg
                .retain(|&(bucket_num, size)| bucket_num > 0 && size < POOL_MAX_SIZE);
            self.cfg
                .sort_unstable_by(|(_, sz0), (_, sz1)| sz0.partial_cmp(sz1).unwrap());
            self.cfg.len()
        }
    }

    /// Pool implementation providing sub-pools with fixed size memory blocks.
    ///
    /// This is a simple memory pool implementation which pre-allocates all subpools using a given
    /// pool configuration. After the pre-allocation, no dynamic memory allocation will be
    /// performed during run-time. This makes the implementation suitable for real-time
    /// applications and embedded environments.
    ///
    /// The subpool bucket sizes only denote the maximum possible data size being stored inside
    /// them and the pool implementation will still track the size of the data stored inside it.
    /// The implementation will generally determine the best fitting subpool for given data to
    /// add. Currently, the pool does not support spilling to larger subpools if the closest
    /// fitting subpool is full. This might be added in the future.
    ///
    /// Transactions with the [pool][StaticMemoryPool] are done using a generic
    /// [address][PoolAddr] type. Adding any data to the pool will yield a store address.
    /// Modification and read operations are done using a reference to a store address. Deletion
    /// will consume the store address.
    pub struct StaticMemoryPool {
        pool_cfg: StaticPoolConfig,
        pool: Vec<Vec<u8>>,
        sizes_lists: Vec<Vec<PoolSize>>,
    }

    impl StaticMemoryPool {
        /// Create a new local pool from the [given configuration][StaticPoolConfig]. This function
        /// will sanitize the given configuration as well.
        pub fn new(mut cfg: StaticPoolConfig) -> StaticMemoryPool {
            let subpools_num = cfg.sanitize();
            let mut local_pool = StaticMemoryPool {
                pool_cfg: cfg,
                pool: Vec::with_capacity(subpools_num),
                sizes_lists: Vec::with_capacity(subpools_num),
            };
            for &(num_elems, elem_size) in local_pool.pool_cfg.cfg.iter() {
                let next_pool_len = elem_size * num_elems as usize;
                local_pool.pool.push(vec![0; next_pool_len]);
                let next_sizes_list_len = num_elems as usize;
                local_pool
                    .sizes_lists
                    .push(vec![STORE_FREE; next_sizes_list_len]);
            }
            local_pool
        }

        fn addr_check(&self, addr: &StaticPoolAddr) -> Result<usize, PoolError> {
            self.validate_addr(addr)?;
            let pool_idx = addr.pool_idx as usize;
            let size_list = self.sizes_lists.get(pool_idx).unwrap();
            let curr_size = size_list[addr.packet_idx as usize];
            if curr_size == STORE_FREE {
                return Err(PoolError::DataDoesNotExist(PoolAddr::from(*addr)));
            }
            Ok(curr_size)
        }

        fn validate_addr(&self, addr: &StaticPoolAddr) -> Result<(), PoolError> {
            let pool_idx = addr.pool_idx as usize;
            if pool_idx >= self.pool_cfg.cfg.len() {
                return Err(PoolError::InvalidStoreId(
                    StoreIdError::InvalidSubpool(addr.pool_idx),
                    Some(PoolAddr::from(*addr)),
                ));
            }
            if addr.packet_idx >= self.pool_cfg.cfg[addr.pool_idx as usize].0 {
                return Err(PoolError::InvalidStoreId(
                    StoreIdError::InvalidPacketIdx(addr.packet_idx),
                    Some(PoolAddr::from(*addr)),
                ));
            }
            Ok(())
        }

        fn reserve(&mut self, data_len: usize) -> Result<StaticPoolAddr, PoolError> {
            let mut subpool_idx = self.find_subpool(data_len, 0)?;

            if self.pool_cfg.spill_to_higher_subpools {
                while let Err(PoolError::StoreFull(_)) = self.find_empty(subpool_idx) {
                    if (subpool_idx + 1) as usize == self.sizes_lists.len() {
                        return Err(PoolError::StoreFull(subpool_idx));
                    }
                    subpool_idx += 1;
                }
            }

            let (slot, size_slot_ref) = self.find_empty(subpool_idx)?;
            *size_slot_ref = data_len;
            Ok(StaticPoolAddr {
                pool_idx: subpool_idx,
                packet_idx: slot,
            })
        }

        fn find_subpool(&self, req_size: usize, start_at_subpool: u16) -> Result<u16, PoolError> {
            for (i, &(_, elem_size)) in self.pool_cfg.cfg.iter().enumerate() {
                if i < start_at_subpool as usize {
                    continue;
                }
                if elem_size >= req_size {
                    return Ok(i as u16);
                }
            }
            Err(PoolError::DataTooLarge(req_size))
        }

        fn write(&mut self, addr: &StaticPoolAddr, data: &[u8]) -> Result<(), PoolError> {
            let packet_pos = self.raw_pos(addr).ok_or(PoolError::InternalError(0))?;
            let subpool = self
                .pool
                .get_mut(addr.pool_idx as usize)
                .ok_or(PoolError::InternalError(1))?;
            let pool_slice = &mut subpool[packet_pos..packet_pos + data.len()];
            pool_slice.copy_from_slice(data);
            Ok(())
        }

        fn find_empty(&mut self, subpool: u16) -> Result<(u16, &mut usize), PoolError> {
            if let Some(size_list) = self.sizes_lists.get_mut(subpool as usize) {
                for (i, elem_size) in size_list.iter_mut().enumerate() {
                    if *elem_size == STORE_FREE {
                        return Ok((i as u16, elem_size));
                    }
                }
            } else {
                return Err(PoolError::InvalidStoreId(
                    StoreIdError::InvalidSubpool(subpool),
                    None,
                ));
            }
            Err(PoolError::StoreFull(subpool))
        }

        fn raw_pos(&self, addr: &StaticPoolAddr) -> Option<usize> {
            let (_, size) = self.pool_cfg.cfg.get(addr.pool_idx as usize)?;
            Some(addr.packet_idx as usize * size)
        }
    }

    impl PoolProvider for StaticMemoryPool {
        fn add(&mut self, data: &[u8]) -> Result<PoolAddr, PoolError> {
            let data_len = data.len();
            if data_len > POOL_MAX_SIZE {
                return Err(PoolError::DataTooLarge(data_len));
            }
            let addr = self.reserve(data_len)?;
            self.write(&addr, data)?;
            Ok(addr.into())
        }

        fn free_element<W: FnMut(&mut [u8])>(
            &mut self,
            len: usize,
            mut writer: W,
        ) -> Result<PoolAddr, PoolError> {
            if len > POOL_MAX_SIZE {
                return Err(PoolError::DataTooLarge(len));
            }
            let addr = self.reserve(len)?;
            let raw_pos = self.raw_pos(&addr).unwrap();
            let block =
                &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()[raw_pos..raw_pos + len];
            writer(block);
            Ok(addr.into())
        }

        fn modify<U: FnMut(&mut [u8])>(
            &mut self,
            addr: &PoolAddr,
            mut updater: U,
        ) -> Result<(), PoolError> {
            let addr = StaticPoolAddr::from(*addr);
            let curr_size = self.addr_check(&addr)?;
            let raw_pos = self.raw_pos(&addr).unwrap();
            let block = &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()
                [raw_pos..raw_pos + curr_size];
            updater(block);
            Ok(())
        }

        fn read(&self, addr: &PoolAddr, buf: &mut [u8]) -> Result<usize, PoolError> {
            let addr = StaticPoolAddr::from(*addr);
            let curr_size = self.addr_check(&addr)?;
            if buf.len() < curr_size {
                return Err(ByteConversionError::ToSliceTooSmall {
                    found: buf.len(),
                    expected: curr_size,
                }
                .into());
            }
            let raw_pos = self.raw_pos(&addr).unwrap();
            let block =
                &self.pool.get(addr.pool_idx as usize).unwrap()[raw_pos..raw_pos + curr_size];
            //block.copy_from_slice(&src);
            buf[..curr_size].copy_from_slice(block);
            Ok(curr_size)
        }

        fn delete(&mut self, addr: PoolAddr) -> Result<(), PoolError> {
            let addr = StaticPoolAddr::from(addr);
            self.addr_check(&addr)?;
            let block_size = self.pool_cfg.cfg.get(addr.pool_idx as usize).unwrap().1;
            let raw_pos = self.raw_pos(&addr).unwrap();
            let block = &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()
                [raw_pos..raw_pos + block_size];
            let size_list = self.sizes_lists.get_mut(addr.pool_idx as usize).unwrap();
            size_list[addr.packet_idx as usize] = STORE_FREE;
            block.fill(0);
            Ok(())
        }

        fn has_element_at(&self, addr: &PoolAddr) -> Result<bool, PoolError> {
            let addr = StaticPoolAddr::from(*addr);
            self.validate_addr(&addr)?;
            let pool_idx = addr.pool_idx as usize;
            let size_list = self.sizes_lists.get(pool_idx).unwrap();
            let curr_size = size_list[addr.packet_idx as usize];
            if curr_size == STORE_FREE {
                return Ok(false);
            }
            Ok(true)
        }

        fn len_of_data(&self, addr: &PoolAddr) -> Result<usize, PoolError> {
            let addr = StaticPoolAddr::from(*addr);
            self.validate_addr(&addr)?;
            let pool_idx = addr.pool_idx as usize;
            let size_list = self.sizes_lists.get(pool_idx).unwrap();
            let size = size_list[addr.packet_idx as usize];
            Ok(match size {
                STORE_FREE => 0,
                _ => size,
            })
        }
    }

    impl PoolProviderWithGuards for StaticMemoryPool {
        fn modify_with_guard(&mut self, addr: PoolAddr) -> PoolRwGuard<Self> {
            PoolRwGuard::new(self, addr)
        }

        fn read_with_guard(&mut self, addr: PoolAddr) -> PoolGuard<Self> {
            PoolGuard::new(self, addr)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pool::{
        PoolError, PoolGuard, PoolProvider, PoolProviderWithGuards, PoolRwGuard, StaticMemoryPool,
        StaticPoolAddr, StaticPoolConfig, StoreIdError, POOL_MAX_SIZE,
    };
    use std::vec;

    fn basic_small_pool() -> StaticMemoryPool {
        // 4 buckets of 4 bytes, 2 of 8 bytes and 1 of 16 bytes
        let pool_cfg = StaticPoolConfig::new(vec![(4, 4), (2, 8), (1, 16)], false);
        StaticMemoryPool::new(pool_cfg)
    }

    #[test]
    fn test_cfg() {
        // Values where number of buckets is 0 or size is too large should be removed
        let mut pool_cfg = StaticPoolConfig::new(vec![(0, 0), (1, 0), (2, POOL_MAX_SIZE)], false);
        pool_cfg.sanitize();
        assert_eq!(*pool_cfg.cfg(), vec![(1, 0)]);
        // Entries should be ordered according to bucket size
        pool_cfg = StaticPoolConfig::new(vec![(16, 6), (32, 3), (8, 12)], false);
        pool_cfg.sanitize();
        assert_eq!(*pool_cfg.cfg(), vec![(32, 3), (16, 6), (8, 12)]);
        // Unstable sort is used, so order of entries with same block length should not matter
        pool_cfg = StaticPoolConfig::new(vec![(12, 12), (14, 16), (10, 12)], false);
        pool_cfg.sanitize();
        assert!(
            *pool_cfg.cfg() == vec![(12, 12), (10, 12), (14, 16)]
                || *pool_cfg.cfg() == vec![(10, 12), (12, 12), (14, 16)]
        );
    }

    #[test]
    fn test_add_and_read() {
        let mut local_pool = basic_small_pool();
        let mut test_buf: [u8; 16] = [0; 16];
        for (i, val) in test_buf.iter_mut().enumerate() {
            *val = i as u8;
        }
        let mut other_buf: [u8; 16] = [0; 16];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        // Read back data and verify correctness
        let res = local_pool.read(&addr, &mut other_buf);
        assert!(res.is_ok());
        let read_len = res.unwrap();
        assert_eq!(read_len, 16);
        for (i, &val) in other_buf.iter().enumerate() {
            assert_eq!(val, i as u8);
        }
    }

    #[test]
    fn test_add_smaller_than_full_slot() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 12] = [0; 12];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        let res = local_pool
            .read(&addr, &mut [0; 12])
            .expect("Read back failed");
        assert_eq!(res, 12);
    }

    #[test]
    fn test_delete() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 16] = [0; 16];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        // Delete the data
        let res = local_pool.delete(addr);
        assert!(res.is_ok());
        let mut writer = |buf: &mut [u8]| {
            assert_eq!(buf.len(), 12);
        };
        // Verify that the slot is free by trying to get a reference to it
        let res = local_pool.free_element(12, &mut writer);
        assert!(res.is_ok());
        let addr = res.unwrap();
        assert_eq!(
            addr,
            u64::from(StaticPoolAddr {
                pool_idx: 2,
                packet_idx: 0
            })
        );
    }

    #[test]
    fn test_modify() {
        let mut local_pool = basic_small_pool();
        let mut test_buf: [u8; 16] = [0; 16];
        for (i, val) in test_buf.iter_mut().enumerate() {
            *val = i as u8;
        }
        let addr = local_pool.add(&test_buf).expect("Adding data failed");

        {
            // Verify that the slot is free by trying to get a reference to it
            local_pool
                .modify(&addr, &mut |buf: &mut [u8]| {
                    buf[0] = 0;
                    buf[1] = 0x42;
                })
                .expect("Modifying data failed");
        }

        local_pool
            .read(&addr, &mut test_buf)
            .expect("Reading back data failed");
        assert_eq!(test_buf[0], 0);
        assert_eq!(test_buf[1], 0x42);
        assert_eq!(test_buf[2], 2);
        assert_eq!(test_buf[3], 3);
    }

    #[test]
    fn test_consecutive_reservation() {
        let mut local_pool = basic_small_pool();
        // Reserve two smaller blocks consecutively and verify that the third reservation fails
        let res = local_pool.free_element(8, |_| {});
        assert!(res.is_ok());
        let addr0 = res.unwrap();
        let res = local_pool.free_element(8, |_| {});
        assert!(res.is_ok());
        let addr1 = res.unwrap();
        let res = local_pool.free_element(8, |_| {});
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err, PoolError::StoreFull(1));

        // Verify that the two deletions are successful
        assert!(local_pool.delete(addr0).is_ok());
        assert!(local_pool.delete(addr1).is_ok());
    }

    #[test]
    fn test_read_does_not_exist() {
        let local_pool = basic_small_pool();
        // Try to access data which does not exist
        let res = local_pool.read(
            &StaticPoolAddr {
                packet_idx: 0,
                pool_idx: 0,
            }
            .into(),
            &mut [],
        );
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            PoolError::DataDoesNotExist { .. }
        ));
    }

    #[test]
    fn test_store_full() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 16] = [0; 16];
        assert!(local_pool.add(&test_buf).is_ok());
        // The subpool is now full and the call should fail accordingly
        let res = local_pool.add(&test_buf);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(matches!(err, PoolError::StoreFull { .. }));
        if let PoolError::StoreFull(subpool) = err {
            assert_eq!(subpool, 2);
        }
    }

    #[test]
    fn test_invalid_pool_idx() {
        let local_pool = basic_small_pool();
        let addr = StaticPoolAddr {
            pool_idx: 3,
            packet_idx: 0,
        }
        .into();
        let res = local_pool.read(&addr, &mut []);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(matches!(
            err,
            PoolError::InvalidStoreId(StoreIdError::InvalidSubpool(3), Some(_))
        ));
    }

    #[test]
    fn test_invalid_packet_idx() {
        let local_pool = basic_small_pool();
        let addr = StaticPoolAddr {
            pool_idx: 2,
            packet_idx: 1,
        };
        assert_eq!(addr.raw(), 0x00020001);
        let res = local_pool.read(&addr.into(), &mut []);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(matches!(
            err,
            PoolError::InvalidStoreId(StoreIdError::InvalidPacketIdx(1), Some(_))
        ));
    }

    #[test]
    fn test_add_too_large() {
        let mut local_pool = basic_small_pool();
        let data_too_large = [0; 20];
        let res = local_pool.add(&data_too_large);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err, PoolError::DataTooLarge(20));
    }

    #[test]
    fn test_data_too_large_1() {
        let mut local_pool = basic_small_pool();
        let res = local_pool.free_element(POOL_MAX_SIZE + 1, |_| {});
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), PoolError::DataTooLarge(POOL_MAX_SIZE + 1));
    }

    #[test]
    fn test_free_element_too_large() {
        let mut local_pool = basic_small_pool();
        // Try to request a slot which is too large
        let res = local_pool.free_element(20, |_| {});
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), PoolError::DataTooLarge(20));
    }

    #[test]
    fn test_pool_guard_deletion_man_creation() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 16] = [0; 16];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        let read_guard = PoolGuard::new(&mut local_pool, addr);
        drop(read_guard);
        assert!(!local_pool.has_element_at(&addr).expect("Invalid address"));
    }

    #[test]
    fn test_pool_guard_deletion() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 16] = [0; 16];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        let read_guard = local_pool.read_with_guard(addr);
        drop(read_guard);
        assert!(!local_pool.has_element_at(&addr).expect("Invalid address"));
    }

    #[test]
    fn test_pool_guard_with_release() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 16] = [0; 16];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        let mut read_guard = PoolGuard::new(&mut local_pool, addr);
        read_guard.release();
        drop(read_guard);
        assert!(local_pool.has_element_at(&addr).expect("Invalid address"));
    }

    #[test]
    fn test_pool_modify_guard_man_creation() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 16] = [0; 16];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        let mut rw_guard = PoolRwGuard::new(&mut local_pool, addr);
        rw_guard.update(&mut |_| {}).expect("modify failed");
        drop(rw_guard);
        assert!(!local_pool.has_element_at(&addr).expect("Invalid address"));
    }

    #[test]
    fn test_pool_modify_guard() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 16] = [0; 16];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        let mut rw_guard = local_pool.modify_with_guard(addr);
        rw_guard.update(&mut |_| {}).expect("modify failed");
        drop(rw_guard);
        assert!(!local_pool.has_element_at(&addr).expect("Invalid address"));
    }

    #[test]
    fn modify_pool_index_above_0() {
        let mut local_pool = basic_small_pool();
        let test_buf_0: [u8; 4] = [1; 4];
        let test_buf_1: [u8; 4] = [2; 4];
        let test_buf_2: [u8; 4] = [3; 4];
        let test_buf_3: [u8; 4] = [4; 4];
        let addr0 = local_pool.add(&test_buf_0).expect("Adding data failed");
        let addr1 = local_pool.add(&test_buf_1).expect("Adding data failed");
        let addr2 = local_pool.add(&test_buf_2).expect("Adding data failed");
        let addr3 = local_pool.add(&test_buf_3).expect("Adding data failed");
        local_pool
            .modify(&addr0, |buf| {
                assert_eq!(buf, test_buf_0);
            })
            .expect("Modifying data failed");
        local_pool
            .modify(&addr1, |buf| {
                assert_eq!(buf, test_buf_1);
            })
            .expect("Modifying data failed");
        local_pool
            .modify(&addr2, |buf| {
                assert_eq!(buf, test_buf_2);
            })
            .expect("Modifying data failed");
        local_pool
            .modify(&addr3, |buf| {
                assert_eq!(buf, test_buf_3);
            })
            .expect("Modifying data failed");
    }

    #[test]
    fn test_spills_to_higher_subpools() {
        let pool_cfg = StaticPoolConfig::new(vec![(2, 8), (2, 16)], true);
        let mut local_pool = StaticMemoryPool::new(pool_cfg);
        local_pool.free_element(8, |_| {}).unwrap();
        local_pool.free_element(8, |_| {}).unwrap();
        let mut in_larger_subpool_now = local_pool.free_element(8, |_| {});
        assert!(in_larger_subpool_now.is_ok());
        let generic_addr = in_larger_subpool_now.unwrap();
        let pool_addr = StaticPoolAddr::from(generic_addr);
        assert_eq!(pool_addr.pool_idx, 1);
        assert_eq!(pool_addr.packet_idx, 0);
        assert!(local_pool.has_element_at(&generic_addr).unwrap());
        in_larger_subpool_now = local_pool.free_element(8, |_| {});
        assert!(in_larger_subpool_now.is_ok());
        let generic_addr = in_larger_subpool_now.unwrap();
        let pool_addr = StaticPoolAddr::from(generic_addr);
        assert_eq!(pool_addr.pool_idx, 1);
        assert_eq!(pool_addr.packet_idx, 1);
        assert!(local_pool.has_element_at(&generic_addr).unwrap());
    }

    #[test]
    fn test_spillage_fails_as_well() {
        let pool_cfg = StaticPoolConfig::new(vec![(1, 8), (1, 16)], true);
        let mut local_pool = StaticMemoryPool::new(pool_cfg);
        local_pool.free_element(8, |_| {}).unwrap();
        local_pool.free_element(8, |_| {}).unwrap();
        let should_fail = local_pool.free_element(8, |_| {});
        assert!(should_fail.is_err());
        if let Err(err) = should_fail {
            assert_eq!(err, PoolError::StoreFull(1));
        } else {
            panic!("unexpected store address");
        }
    }

    #[test]
    fn test_spillage_works_across_multiple_subpools() {
        let pool_cfg = StaticPoolConfig::new(vec![(1, 8), (1, 12), (1, 16)], true);
        let mut local_pool = StaticMemoryPool::new(pool_cfg);
        local_pool.free_element(8, |_| {}).unwrap();
        local_pool.free_element(12, |_| {}).unwrap();
        let in_larger_subpool_now = local_pool.free_element(8, |_| {});
        assert!(in_larger_subpool_now.is_ok());
        let generic_addr = in_larger_subpool_now.unwrap();
        let pool_addr = StaticPoolAddr::from(generic_addr);
        assert_eq!(pool_addr.pool_idx, 2);
        assert_eq!(pool_addr.packet_idx, 0);
        assert!(local_pool.has_element_at(&generic_addr).unwrap());
    }

    #[test]
    fn test_spillage_fails_across_multiple_subpools() {
        let pool_cfg = StaticPoolConfig::new(vec![(1, 8), (1, 12), (1, 16)], true);
        let mut local_pool = StaticMemoryPool::new(pool_cfg);
        local_pool.free_element(8, |_| {}).unwrap();
        local_pool.free_element(12, |_| {}).unwrap();
        local_pool.free_element(16, |_| {}).unwrap();
        let should_fail = local_pool.free_element(8, |_| {});
        assert!(should_fail.is_err());
        if let Err(err) = should_fail {
            assert_eq!(err, PoolError::StoreFull(2));
        } else {
            panic!("unexpected store address");
        }
    }
}
