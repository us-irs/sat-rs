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

#[derive(Debug)]
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
            local_pool.sizes_lists.push(vec![Self::STORE_FREE; next_sizes_list_len]);
        }
        local_pool
    }

    pub fn add(&mut self, data: &[u8]) -> Result<StoreAddr, StoreError> {
        if data.len() > Self::MAX_SIZE {
            return Err(StoreError::DataTooLarge(data.len()));
        }
        self.reserve(data)?;
        let addr = StoreAddr {
            packet_idx: 0,
            pool_idx: 0,
        };
        self.write(&addr, data)?;
        Ok(addr)
    }

    pub fn free_element(&mut self) -> Result<(StoreAddr, &mut [u8]), StoreError> {
        Ok((
            StoreAddr {
                packet_idx: 0,
                pool_idx: 0,
            },
            self.pool[0].as_mut_slice(),
        ))
    }

    pub fn get(&self, _addr: StoreAddr) -> Result<&[u8], StoreError> {
        Ok(self.pool[0].as_slice())
    }

    pub fn modify(&mut self, _addr: StoreAddr) -> Result<&mut [u8], StoreError> {
        Ok(self.pool[0].as_mut_slice())
    }

    pub fn delete(&mut self, _addr: StoreAddr) -> Result<(), StoreError> {
        Ok(())
    }

    fn reserve(&mut self, data: &[u8]) -> Result<StoreAddr, StoreError> {
        let subpool_idx = self.find_subpool(data.len(), 0)?;
        let (slot, size_slot_ref) = self.find_empty(subpool_idx)?;
        *size_slot_ref = data.len();
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
        let packet_pos = self
            .raw_pos(addr)
            .ok_or_else(||{
                StoreError::InternalError(format!(
                    "write: Error in raw_pos func with address {:?}",
                    addr
                ))
            })?;
        let subpool =
            self.pool
                .get_mut(addr.pool_idx as usize)
                .ok_or_else(||{
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
