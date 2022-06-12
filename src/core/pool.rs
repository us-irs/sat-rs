type NumBuckets = u16;

pub struct PoolCfg {
    cfg: Vec<(NumBuckets, usize)>,
}

impl PoolCfg {
    pub fn new(cfg: Vec<(NumBuckets, usize)>) -> Self {
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

pub struct LocalPool {
    pool_cfg: PoolCfg,
    pool: Vec<Vec<u8>>,
    sizes_lists: Vec<Vec<PoolSize>>,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct StoreAddr {
    pool_idx: u16,
    packet_idx: NumBuckets,
}

impl StoreAddr {
    pub const INVALID_ADDR: u32 = 0xFFFFFFFF;

    pub fn raw(&self) -> u32 {
        ((self.pool_idx as u32) << 16) as u32 | self.packet_idx as u32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StoreIdError {
    InvalidSubpool(u16),
    InvalidPacketIdx(u16)
}

#[derive(Debug, Clone, PartialEq)]
pub enum StoreError {
    /// Requested data block is too large
    DataTooLarge(usize),
    /// The store is full. Contains the faulty subpool
    StoreFull(u16),
    /// Store ID is invalid. This also includes partial errors where only the subpool is invalid
    InvalidStoreId(StoreIdError, Option<StoreAddr>),
    /// Valid subpool and packet index, but no data is stored at the given address
    DataDoesNotExist(StoreAddr),
    /// Internal or configuration errors
    InternalError(String),
}

impl LocalPool {
    const STORE_FREE: PoolSize = PoolSize::MAX;
    const MAX_SIZE: PoolSize = Self::STORE_FREE - 1;

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

    pub fn add(&mut self, data: &[u8]) -> Result<StoreAddr, StoreError> {
        if data.len() > Self::MAX_SIZE {
            return Err(StoreError::DataTooLarge(data.len()));
        }
        let addr = self.reserve(data.len())?;
        self.write(&addr, data)?;
        Ok(addr)
    }

    pub fn free_element(&mut self, len: usize) -> Result<(StoreAddr, &mut [u8]), StoreError> {
        if len > Self::MAX_SIZE {
            return Err(StoreError::DataTooLarge(len));
        }
        let addr = self.reserve(len)?;
        let raw_pos = self.raw_pos(&addr).unwrap();
        let block = &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()[raw_pos..len];
        Ok((addr, block))
    }

    pub fn modify(&mut self, addr: &StoreAddr) -> Result<&mut [u8], StoreError> {
        let curr_size = self.addr_check(&addr)?;
        let raw_pos = self.raw_pos(&addr).unwrap();
        let block = &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()[raw_pos..curr_size];
        Ok(block)
    }

    pub fn read(&self, addr: &StoreAddr) -> Result<&[u8], StoreError> {
        let curr_size = self.addr_check(&addr)?;
        let raw_pos = self.raw_pos(&addr).unwrap();
        let block = &self.pool.get(addr.pool_idx as usize).unwrap()[raw_pos..curr_size];
        Ok(block)
    }

    pub fn delete(&mut self, addr: StoreAddr) -> Result<(), StoreError> {
        self.addr_check(&addr)?;
        let block_size = self.pool_cfg.cfg.get(addr.pool_idx as usize).unwrap().1;
        let raw_pos = self.raw_pos(&addr).unwrap();
        let block = &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()[raw_pos..block_size];
        let size_list = self.sizes_lists.get_mut(addr.pool_idx as usize).unwrap();
        size_list[addr.packet_idx as usize] = Self::STORE_FREE;
        block.fill(0);
        Ok(())
    }

    fn addr_check(&self, addr: &StoreAddr) -> Result<usize, StoreError> {
        let pool_idx = addr.pool_idx as usize;
        if pool_idx as usize >= self.pool_cfg.cfg.len() {
            return Err(StoreError::InvalidStoreId(StoreIdError::InvalidSubpool(addr.pool_idx), Some(*addr)));
        }
        if addr.packet_idx >= self.pool_cfg.cfg[addr.pool_idx as usize].0 {
            return Err(StoreError::InvalidStoreId(StoreIdError::InvalidPacketIdx(addr.packet_idx), Some(*addr)));
        }
        let size_list = self.sizes_lists.get(pool_idx).unwrap();
        let curr_size = size_list[addr.packet_idx as usize];
        if curr_size == Self::STORE_FREE {
            return Err(StoreError::DataDoesNotExist(*addr));
        }
        Ok(curr_size)
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
        let pool_slice = &mut subpool[packet_pos..self.pool_cfg.cfg[addr.pool_idx as usize].1];
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
            return Err(StoreError::InvalidStoreId(StoreIdError::InvalidSubpool(subpool),None));
        }
        Err(StoreError::StoreFull(subpool))
    }

    fn raw_pos(&self, addr: &StoreAddr) -> Option<usize> {
        let (_, size) = self.pool_cfg.cfg.get(addr.pool_idx as usize)?;
        Some(addr.packet_idx as usize * size)
    }
}

#[cfg(test)]
mod tests {
    use crate::core::pool::{LocalPool, PoolCfg, StoreAddr, StoreError, StoreIdError};

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
    fn test_basic() {
        // 4 buckets of 4 bytes, 2 of 8 bytes and 1 of 16 bytes
        let pool_cfg = PoolCfg::new(vec![(4, 4), (2, 8), (1, 16)]);
        let mut local_pool = LocalPool::new(pool_cfg);
        // Try to access data which does not exist
        let res = local_pool.read(&StoreAddr{
            packet_idx: 0,
            pool_idx: 0
        });
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), StoreError::DataDoesNotExist { .. }));
        let mut test_buf: [u8; 16] = [0; 16];
        for (i, val) in test_buf.iter_mut().enumerate() {
            *val = i as u8;
        }
        let res = local_pool.add(test_buf.as_slice());
        assert!(res.is_ok());
        let addr = res.unwrap();
        // Only the second subpool has enough storage and only one bucket
        assert_eq!(addr, StoreAddr {
            pool_idx: 2,
            packet_idx: 0
        });

        // The subpool is now full and the call should fail accordingly
        let res = local_pool.add(test_buf.as_slice());
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(matches!(err, StoreError::StoreFull {..}));
        if let StoreError::StoreFull(subpool) = err {
            assert_eq!(subpool, 2);
        }

        // Read back data and verify correctness
        let res = local_pool.read(&addr);
        assert!(res.is_ok());
        let buf_read_back = res.unwrap();
        assert_eq!(buf_read_back.len(), 16);
        for (i, &val) in buf_read_back.iter().enumerate() {
            assert_eq!(val , i as u8);
        }

        // Delete the data
        let res = local_pool.delete(addr);
        assert!(res.is_ok());

        {
            // Verify that the slot is free by trying to get a reference to it
            let res = local_pool.free_element(12);
            assert!(res.is_ok());
            let (addr, buf_ref) = res.unwrap();
            assert_eq!(addr, StoreAddr {pool_idx: 2, packet_idx:0});
            assert_eq!(buf_ref.len(), 12);
            assert_eq!(buf_ref, [0; 12]);
            buf_ref[0] = 5;
            buf_ref[11] = 12;
        }

        {
            // Try to request a slot which is too large
            let res = local_pool.free_element(20);
            assert!(res.is_err());
            assert_eq!(res.unwrap_err(), StoreError::DataTooLarge(20));

            // Try to modify the 12 bytes requested previously
            let res = local_pool.modify(&addr);
            assert!(res.is_ok());
            let buf_ref = res.unwrap();
            assert_eq!(buf_ref[0], 5);
            assert_eq!(buf_ref[11], 12);
            buf_ref[0] = 0;
            buf_ref[11] = 0;
        }

        {
            let addr = StoreAddr{pool_idx: 3, packet_idx: 0};
            let res = local_pool.read(&addr);
            assert!(res.is_err());
            let err = res.unwrap_err();
            assert!(matches!(err, StoreError::InvalidStoreId(StoreIdError::InvalidSubpool(3), Some(_))));
        }

        {
            let addr = StoreAddr{pool_idx: 2, packet_idx: 1};
            assert_eq!(addr.raw(), 0x00020001);
            let res = local_pool.read(&addr);
            assert!(res.is_err());
            let err = res.unwrap_err();
            assert!(matches!(err, StoreError::InvalidStoreId(StoreIdError::InvalidPacketIdx(1), Some(_))));

            let data_too_large = [0; 20];
            let res = local_pool.add(data_too_large.as_slice());
            assert!(res.is_err());
            let err = res.unwrap_err();
            assert_eq!(err, StoreError::DataTooLarge(20));

            let res = local_pool.free_element(LocalPool::MAX_SIZE + 1);
            assert!(res.is_err());
            assert_eq!(res.unwrap_err(), StoreError::DataTooLarge(LocalPool::MAX_SIZE + 1));
        }

        {
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
    }
}
