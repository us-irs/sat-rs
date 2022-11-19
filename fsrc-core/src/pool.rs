//! # Pool implementation providing pre-allocated sub-pools with fixed size memory blocks
//!
//! This is a simple memory pool implementation which pre-allocates all sub-pools using a given pool
//! configuration. After the pre-allocation, no dynamic memory allocation will be performed
//! during run-time. This makes the implementation suitable for real-time applications and
//! embedded environments. The pool implementation will also track the size of the data stored
//! inside it.
//!
//! Transactions with the [pool][LocalPool] are done using a special [address][StoreAddr] type.
//! Adding any data to the pool will yield a store address. Modification and read operations are
//! done using a reference to a store address. Deletion will consume the store address.
//!
//! # Example
//!
//! ```
//! use fsrc_core::pool::{LocalPool, PoolCfg, PoolProvider};
//!
//! // 4 buckets of 4 bytes, 2 of 8 bytes and 1 of 16 bytes
//! let pool_cfg = PoolCfg::new(vec![(4, 4), (2, 8), (1, 16)]);
//! let mut local_pool = LocalPool::new(pool_cfg);
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
//!     let res = local_pool.read(&addr);
//!     assert!(res.is_ok());
//!     let buf_read_back = res.unwrap();
//!     assert_eq!(buf_read_back.len(), 4);
//!     assert_eq!(buf_read_back[0], 42);
//!     // Modify the stored data
//!     let res = local_pool.modify(&addr);
//!     assert!(res.is_ok());
//!     let buf_read_back = res.unwrap();
//!     buf_read_back[0] = 12;
//! }
//!
//! {
//!     // Read the modified data back
//!     let res = local_pool.read(&addr);
//!     assert!(res.is_ok());
//!     let buf_read_back = res.unwrap();
//!     assert_eq!(buf_read_back.len(), 4);
//!     assert_eq!(buf_read_back[0], 12);
//! }
//!
//! // Delete the stored data
//! local_pool.delete(addr);
//!
//! // Get a free element in the pool with an appropriate size
//! {
//!     let res = local_pool.free_element(12);
//!     assert!(res.is_ok());
//!     let (tmp, mut_buf) = res.unwrap();
//!     addr = tmp;
//!     mut_buf[0] = 7;
//! }
//!
//! // Read back the data
//! {
//!     // Read the store data back
//!     let res = local_pool.read(&addr);
//!     assert!(res.is_ok());
//!     let buf_read_back = res.unwrap();
//!     assert_eq!(buf_read_back.len(), 12);
//!     assert_eq!(buf_read_back[0], 7);
//! }
//! ```
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use delegate::delegate;
#[cfg(feature = "std")]
use std::boxed::Box;
#[cfg(feature = "std")]
use std::sync::{Arc, RwLock};

type NumBlocks = u16;

#[cfg(feature = "std")]
pub type ShareablePoolProvider = Box<dyn PoolProvider + Send + Sync>;
#[cfg(feature = "std")]
pub type SharedPool = Arc<RwLock<ShareablePoolProvider>>;

/// Configuration structure of the [local pool][LocalPool]
///
/// # Parameters
///
/// * `cfg`: Vector of tuples which represent a subpool. The first entry in the tuple specifies the
///       number of memory blocks in the subpool, the second entry the size of the blocks
#[derive(Clone)]
pub struct PoolCfg {
    cfg: Vec<(NumBlocks, usize)>,
}

impl PoolCfg {
    pub fn new(cfg: Vec<(NumBlocks, usize)>) -> Self {
        PoolCfg { cfg }
    }

    pub fn sanitize(&mut self) -> usize {
        self.cfg
            .retain(|&(bucket_num, size)| bucket_num > 0 && size < LocalPool::MAX_SIZE);
        self.cfg
            .sort_unstable_by(|(_, sz0), (_, sz1)| sz0.partial_cmp(sz1).unwrap());
        self.cfg.len()
    }
}

type PoolSize = usize;

/// Pool implementation providing sub-pools with fixed size memory blocks. More details in
/// the [module documentation][super::pool]
pub struct LocalPool {
    pool_cfg: PoolCfg,
    pool: Vec<Vec<u8>>,
    sizes_lists: Vec<Vec<PoolSize>>,
}

/// Simple address type used for transactions with the local pool.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct StoreAddr {
    pool_idx: u16,
    packet_idx: NumBlocks,
}

impl StoreAddr {
    pub const INVALID_ADDR: u32 = 0xFFFFFFFF;

    pub fn raw(&self) -> u32 {
        ((self.pool_idx as u32) << 16) as u32 | self.packet_idx as u32
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreIdError {
    InvalidSubpool(u16),
    InvalidPacketIdx(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Requested data block is too large
    DataTooLarge(usize),
    /// The store is full. Contains the index of the full subpool
    StoreFull(u16),
    /// Store ID is invalid. This also includes partial errors where only the subpool is invalid
    InvalidStoreId(StoreIdError, Option<StoreAddr>),
    /// Valid subpool and packet index, but no data is stored at the given address
    DataDoesNotExist(StoreAddr),
    /// Internal or configuration errors
    InternalError(String),
}

pub trait PoolProvider {
    /// Add new data to the pool. The provider should attempt to reserve a memory block with the
    /// appropriate size and then copy the given data to the block. Yields a [StoreAddr] which can
    /// be used to access the data stored in the pool
    fn add(&mut self, data: &[u8]) -> Result<StoreAddr, StoreError>;

    /// The provider should attempt to reserve a free memory block with the appropriate size and
    /// then return a mutable reference to it. Yields a [StoreAddr] which can be used to access
    /// the data stored in the pool
    fn free_element(&mut self, len: usize) -> Result<(StoreAddr, &mut [u8]), StoreError>;

    /// Modify data added previously using a given [StoreAddr] by yielding a mutable reference
    /// to it
    fn modify(&mut self, addr: &StoreAddr) -> Result<&mut [u8], StoreError>;

    /// This function behaves like [Self::modify], but consumes the provided address and returns a
    /// RAII conformant guard object.
    ///
    /// Unless the guard [PoolRwGuard::release] method is called, the data for the
    /// given address will be deleted automatically when the guard is dropped.
    /// This can prevent memory leaks. Users can read (and modify) the data and release the guard
    /// if the data in the store is valid for further processing. If the data is faulty, no
    /// manual deletion is necessary when returning from a processing function prematurely.
    fn modify_with_guard(&mut self, addr: StoreAddr) -> PoolRwGuard;

    /// Read data by yielding a read-only reference given a [StoreAddr]
    fn read(&self, addr: &StoreAddr) -> Result<&[u8], StoreError>;

    /// This function behaves like [Self::read], but consumes the provided address and returns a
    /// RAII conformant guard object.
    ///
    /// Unless the guard [PoolRwGuard::release] method is called, the data for the
    /// given address will be deleted automatically when the guard is dropped.
    /// This can prevent memory leaks. Users can read the data and release the guard
    /// if the data in the store is valid for further processing. If the data is faulty, no
    /// manual deletion is necessary when returning from a processing function prematurely.
    fn read_with_guard(&mut self, addr: StoreAddr) -> PoolGuard;

    /// Delete data inside the pool given a [StoreAddr]
    fn delete(&mut self, addr: StoreAddr) -> Result<(), StoreError>;
    fn has_element_at(&self, addr: &StoreAddr) -> Result<bool, StoreError>;
}

impl LocalPool {
    const STORE_FREE: PoolSize = PoolSize::MAX;
    const MAX_SIZE: PoolSize = Self::STORE_FREE - 1;
    /// Create a new local pool from the [given configuration][PoolCfg]. This function will sanitize
    /// the given configuration as well.
    pub fn new(mut cfg: PoolCfg) -> LocalPool {
        let subpools_num = cfg.sanitize();
        let mut local_pool = LocalPool {
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
                .push(vec![Self::STORE_FREE; next_sizes_list_len]);
        }
        local_pool
    }

    fn addr_check(&self, addr: &StoreAddr) -> Result<usize, StoreError> {
        self.validate_addr(addr)?;
        let pool_idx = addr.pool_idx as usize;
        let size_list = self.sizes_lists.get(pool_idx).unwrap();
        let curr_size = size_list[addr.packet_idx as usize];
        if curr_size == Self::STORE_FREE {
            return Err(StoreError::DataDoesNotExist(*addr));
        }
        Ok(curr_size)
    }

    fn validate_addr(&self, addr: &StoreAddr) -> Result<(), StoreError> {
        let pool_idx = addr.pool_idx as usize;
        if pool_idx as usize >= self.pool_cfg.cfg.len() {
            return Err(StoreError::InvalidStoreId(
                StoreIdError::InvalidSubpool(addr.pool_idx),
                Some(*addr),
            ));
        }
        if addr.packet_idx >= self.pool_cfg.cfg[addr.pool_idx as usize].0 {
            return Err(StoreError::InvalidStoreId(
                StoreIdError::InvalidPacketIdx(addr.packet_idx),
                Some(*addr),
            ));
        }
        Ok(())
    }

    fn reserve(&mut self, data_len: usize) -> Result<StoreAddr, StoreError> {
        let subpool_idx = self.find_subpool(data_len, 0)?;
        let (slot, size_slot_ref) = self.find_empty(subpool_idx)?;
        *size_slot_ref = data_len;
        Ok(StoreAddr {
            pool_idx: subpool_idx,
            packet_idx: slot,
        })
    }

    fn find_subpool(&self, req_size: usize, start_at_subpool: u16) -> Result<u16, StoreError> {
        for (i, &(_, elem_size)) in self.pool_cfg.cfg.iter().enumerate() {
            if i < start_at_subpool as usize {
                continue;
            }
            if elem_size >= req_size {
                return Ok(i as u16);
            }
        }
        Err(StoreError::DataTooLarge(req_size))
    }

    fn write(&mut self, addr: &StoreAddr, data: &[u8]) -> Result<(), StoreError> {
        let packet_pos = self.raw_pos(addr).ok_or_else(|| {
            StoreError::InternalError(format!(
                "write: Error in raw_pos func with address {:?}",
                addr
            ))
        })?;
        let subpool = self.pool.get_mut(addr.pool_idx as usize).ok_or_else(|| {
            StoreError::InternalError(format!(
                "write: Error retrieving pool slice with address {:?}",
                addr
            ))
        })?;
        let pool_slice = &mut subpool[packet_pos..packet_pos + data.len()];
        pool_slice.copy_from_slice(data);
        Ok(())
    }

    fn find_empty(&mut self, subpool: u16) -> Result<(u16, &mut usize), StoreError> {
        if let Some(size_list) = self.sizes_lists.get_mut(subpool as usize) {
            for (i, elem_size) in size_list.iter_mut().enumerate() {
                if *elem_size == Self::STORE_FREE {
                    return Ok((i as u16, elem_size));
                }
            }
        } else {
            return Err(StoreError::InvalidStoreId(
                StoreIdError::InvalidSubpool(subpool),
                None,
            ));
        }
        Err(StoreError::StoreFull(subpool))
    }

    fn raw_pos(&self, addr: &StoreAddr) -> Option<usize> {
        let (_, size) = self.pool_cfg.cfg.get(addr.pool_idx as usize)?;
        Some(addr.packet_idx as usize * size)
    }
}

impl PoolProvider for LocalPool {
    fn add(&mut self, data: &[u8]) -> Result<StoreAddr, StoreError> {
        let data_len = data.len();
        if data_len > Self::MAX_SIZE {
            return Err(StoreError::DataTooLarge(data_len));
        }
        let addr = self.reserve(data_len)?;
        self.write(&addr, data)?;
        Ok(addr)
    }

    fn free_element(&mut self, len: usize) -> Result<(StoreAddr, &mut [u8]), StoreError> {
        if len > Self::MAX_SIZE {
            return Err(StoreError::DataTooLarge(len));
        }
        let addr = self.reserve(len)?;
        let raw_pos = self.raw_pos(&addr).unwrap();
        let block = &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()[raw_pos..raw_pos + len];
        Ok((addr, block))
    }

    fn modify(&mut self, addr: &StoreAddr) -> Result<&mut [u8], StoreError> {
        let curr_size = self.addr_check(addr)?;
        let raw_pos = self.raw_pos(addr).unwrap();
        let block = &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()[raw_pos..curr_size];
        Ok(block)
    }

    fn modify_with_guard(&mut self, addr: StoreAddr) -> PoolRwGuard {
        PoolRwGuard::new(self, addr)
    }

    fn read(&self, addr: &StoreAddr) -> Result<&[u8], StoreError> {
        let curr_size = self.addr_check(addr)?;
        let raw_pos = self.raw_pos(addr).unwrap();
        let block = &self.pool.get(addr.pool_idx as usize).unwrap()[raw_pos..raw_pos + curr_size];
        Ok(block)
    }

    fn read_with_guard(&mut self, addr: StoreAddr) -> PoolGuard {
        PoolGuard::new(self, addr)
    }

    fn delete(&mut self, addr: StoreAddr) -> Result<(), StoreError> {
        self.addr_check(&addr)?;
        let block_size = self.pool_cfg.cfg.get(addr.pool_idx as usize).unwrap().1;
        let raw_pos = self.raw_pos(&addr).unwrap();
        let block =
            &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()[raw_pos..raw_pos + block_size];
        let size_list = self.sizes_lists.get_mut(addr.pool_idx as usize).unwrap();
        size_list[addr.packet_idx as usize] = Self::STORE_FREE;
        block.fill(0);
        Ok(())
    }

    fn has_element_at(&self, addr: &StoreAddr) -> Result<bool, StoreError> {
        self.validate_addr(addr)?;
        let pool_idx = addr.pool_idx as usize;
        let size_list = self.sizes_lists.get(pool_idx).unwrap();
        let curr_size = size_list[addr.packet_idx as usize];
        if curr_size == Self::STORE_FREE {
            return Ok(false);
        }
        Ok(true)
    }
}

pub struct PoolGuard<'a> {
    pool: &'a mut LocalPool,
    pub addr: StoreAddr,
    no_deletion: bool,
    deletion_failed_error: Option<StoreError>,
}

/// This helper object
impl<'a> PoolGuard<'a> {
    pub fn new(pool: &'a mut LocalPool, addr: StoreAddr) -> Self {
        Self {
            pool,
            addr,
            no_deletion: false,
            deletion_failed_error: None,
        }
    }

    pub fn read(&self) -> Result<&[u8], StoreError> {
        self.pool.read(&self.addr)
    }

    /// Releasing the pool guard will disable the automatic deletion of the data when the guard
    /// is dropped.
    pub fn release(&mut self) {
        self.no_deletion = true;
    }
}

impl Drop for PoolGuard<'_> {
    fn drop(&mut self) {
        if !self.no_deletion {
            if let Err(e) = self.pool.delete(self.addr) {
                self.deletion_failed_error = Some(e);
            }
        }
    }
}

pub struct PoolRwGuard<'a> {
    guard: PoolGuard<'a>,
}

impl<'a> PoolRwGuard<'a> {
    pub fn new(pool: &'a mut LocalPool, addr: StoreAddr) -> Self {
        Self {
            guard: PoolGuard::new(pool, addr),
        }
    }

    pub fn modify(&mut self) -> Result<&mut [u8], StoreError> {
        self.guard.pool.modify(&self.guard.addr)
    }

    delegate!(
        to self.guard {
            pub fn read(&self) -> Result<&[u8], StoreError>;
            /// Releasing the pool guard will disable the automatic deletion of the data when the guard
            /// is dropped.
            pub fn release(&mut self);
        }
    );
}

#[cfg(test)]
mod tests {
    use crate::pool::{
        LocalPool, PoolCfg, PoolGuard, PoolProvider, PoolRwGuard, StoreAddr, StoreError,
        StoreIdError,
    };
    use std::vec;

    fn basic_small_pool() -> LocalPool {
        // 4 buckets of 4 bytes, 2 of 8 bytes and 1 of 16 bytes
        let pool_cfg = PoolCfg::new(vec![(4, 4), (2, 8), (1, 16)]);
        LocalPool::new(pool_cfg)
    }

    #[test]
    fn test_cfg() {
        // Values where number of buckets is 0 or size is too large should be removed
        let mut pool_cfg = PoolCfg::new(vec![(0, 0), (1, 0), (2, LocalPool::MAX_SIZE)]);
        pool_cfg.sanitize();
        assert_eq!(pool_cfg.cfg, vec![(1, 0)]);
        // Entries should be ordered according to bucket size
        pool_cfg = PoolCfg::new(vec![(16, 6), (32, 3), (8, 12)]);
        pool_cfg.sanitize();
        assert_eq!(pool_cfg.cfg, vec![(32, 3), (16, 6), (8, 12)]);
        // Unstable sort is used, so order of entries with same block length should not matter
        pool_cfg = PoolCfg::new(vec![(12, 12), (14, 16), (10, 12)]);
        pool_cfg.sanitize();
        assert!(
            pool_cfg.cfg == vec![(12, 12), (10, 12), (14, 16)]
                || pool_cfg.cfg == vec![(10, 12), (12, 12), (14, 16)]
        );
    }

    #[test]
    fn test_add_and_read() {
        let mut local_pool = basic_small_pool();
        let mut test_buf: [u8; 16] = [0; 16];
        for (i, val) in test_buf.iter_mut().enumerate() {
            *val = i as u8;
        }
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        // Read back data and verify correctness
        let res = local_pool.read(&addr);
        assert!(res.is_ok());
        let buf_read_back = res.unwrap();
        assert_eq!(buf_read_back.len(), 16);
        for (i, &val) in buf_read_back.iter().enumerate() {
            assert_eq!(val, i as u8);
        }
    }

    #[test]
    fn test_add_smaller_than_full_slot() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 12] = [0; 12];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        let res = local_pool.read(&addr).expect("Read back failed");
        assert_eq!(res.len(), 12);
    }

    #[test]
    fn test_delete() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 16] = [0; 16];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        // Delete the data
        let res = local_pool.delete(addr);
        assert!(res.is_ok());
        // Verify that the slot is free by trying to get a reference to it
        let res = local_pool.free_element(12);
        assert!(res.is_ok());
        let (addr, buf_ref) = res.unwrap();
        assert_eq!(
            addr,
            StoreAddr {
                pool_idx: 2,
                packet_idx: 0
            }
        );
        assert_eq!(buf_ref.len(), 12);
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
            let res = local_pool.modify(&addr).expect("Modifying data failed");
            res[0] = 0;
            res[1] = 0x42;
        }

        let res = local_pool.read(&addr).expect("Reading back data failed");
        assert_eq!(res[0], 0);
        assert_eq!(res[1], 0x42);
        assert_eq!(res[2], 2);
        assert_eq!(res[3], 3);
    }

    #[test]
    fn test_consecutive_reservation() {
        let mut local_pool = basic_small_pool();
        // Reserve two smaller blocks consecutively and verify that the third reservation fails
        let res = local_pool.free_element(8);
        assert!(res.is_ok());
        let (addr0, _) = res.unwrap();
        let res = local_pool.free_element(8);
        assert!(res.is_ok());
        let (addr1, _) = res.unwrap();
        let res = local_pool.free_element(8);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err, StoreError::StoreFull(1));

        // Verify that the two deletions are successful
        assert!(local_pool.delete(addr0).is_ok());
        assert!(local_pool.delete(addr1).is_ok());
    }

    #[test]
    fn test_read_does_not_exist() {
        let local_pool = basic_small_pool();
        // Try to access data which does not exist
        let res = local_pool.read(&StoreAddr {
            packet_idx: 0,
            pool_idx: 0,
        });
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            StoreError::DataDoesNotExist { .. }
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
        assert!(matches!(err, StoreError::StoreFull { .. }));
        if let StoreError::StoreFull(subpool) = err {
            assert_eq!(subpool, 2);
        }
    }

    #[test]
    fn test_invalid_pool_idx() {
        let local_pool = basic_small_pool();
        let addr = StoreAddr {
            pool_idx: 3,
            packet_idx: 0,
        };
        let res = local_pool.read(&addr);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(matches!(
            err,
            StoreError::InvalidStoreId(StoreIdError::InvalidSubpool(3), Some(_))
        ));
    }

    #[test]
    fn test_invalid_packet_idx() {
        let local_pool = basic_small_pool();
        let addr = StoreAddr {
            pool_idx: 2,
            packet_idx: 1,
        };
        assert_eq!(addr.raw(), 0x00020001);
        let res = local_pool.read(&addr);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(matches!(
            err,
            StoreError::InvalidStoreId(StoreIdError::InvalidPacketIdx(1), Some(_))
        ));
    }

    #[test]
    fn test_add_too_large() {
        let mut local_pool = basic_small_pool();
        let data_too_large = [0; 20];
        let res = local_pool.add(&data_too_large);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err, StoreError::DataTooLarge(20));
    }

    #[test]
    fn test_data_too_large_1() {
        let mut local_pool = basic_small_pool();
        let res = local_pool.free_element(LocalPool::MAX_SIZE + 1);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err(),
            StoreError::DataTooLarge(LocalPool::MAX_SIZE + 1)
        );
    }

    #[test]
    fn test_free_element_too_large() {
        let mut local_pool = basic_small_pool();
        // Try to request a slot which is too large
        let res = local_pool.free_element(20);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), StoreError::DataTooLarge(20));
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
        let _ = rw_guard.modify().expect("modify failed");
        drop(rw_guard);
        assert!(!local_pool.has_element_at(&addr).expect("Invalid address"));
    }

    #[test]
    fn test_pool_modify_guard() {
        let mut local_pool = basic_small_pool();
        let test_buf: [u8; 16] = [0; 16];
        let addr = local_pool.add(&test_buf).expect("Adding data failed");
        let mut rw_guard = local_pool.modify_with_guard(addr);
        let _ = rw_guard.modify().expect("modify failed");
        drop(rw_guard);
        assert!(!local_pool.has_element_at(&addr).expect("Invalid address"));
    }
}
