type NumBuckets = u16;

pub struct PoolCfg {
    cfg: Vec<(NumBuckets, usize)>,
}

impl PoolCfg {
    pub fn add_pool(&mut self, num_elems: NumBuckets, elem_size: usize) {
        self.cfg.push((num_elems, elem_size))
    }

    fn order(&mut self) -> usize {
        self.cfg.sort_unstable();
        self.cfg.len()
    }
}

type PoolSize = usize;

pub struct LocalPool {
    pool_cfg: PoolCfg,
    pool: Vec<Vec<u8>>,
    sizes_lists: Vec<Vec<PoolSize>>,
}

#[derive(Debug, Copy, Clone)]
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

pub enum StoreError {
    DataTooLarge(usize),
    InvalidSubpool(u16),
    StoreFull(u16),
    InvalidStoreId(StoreAddr),
    DataDoesNotExist(StoreAddr),
    InternalError(String),
}

impl LocalPool {
    const STORE_FREE: PoolSize = PoolSize::MAX;
    const MAX_SIZE: PoolSize = Self::STORE_FREE - 1;

    pub fn new(mut cfg: PoolCfg) -> LocalPool {
        let subpools_num = cfg.order();
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

    pub fn modify(&mut self, addr: StoreAddr) -> Result<&mut [u8], StoreError> {
        let curr_size = self.addr_check(&addr)?;
        let raw_pos = self.raw_pos(&addr).unwrap();
        let block = &mut self.pool.get_mut(addr.pool_idx as usize).unwrap()[raw_pos..curr_size];
        Ok(block)
    }

    pub fn get(&self, addr: StoreAddr) -> Result<&[u8], StoreError> {
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
            return Err(StoreError::InvalidStoreId(*addr));
        }
        if addr.packet_idx >= self.pool_cfg.cfg[addr.pool_idx as usize].0 {
            return Err(StoreError::InvalidStoreId(*addr));
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
            return Err(StoreError::InvalidSubpool(subpool));
        }
        Err(StoreError::StoreFull(subpool))
    }

    fn raw_pos(&self, addr: &StoreAddr) -> Option<usize> {
        let (_, size) = self.pool_cfg.cfg.get(addr.pool_idx as usize)?;
        Some(addr.packet_idx as usize * size)
    }
}
