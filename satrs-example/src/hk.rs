pub type CollectionIntervalFactor = u32;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AcsHkIds {
    TestMgmSet = 1,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HkRequest {
    OneShot(u32),
    Enable(u32, CollectionIntervalFactor),
    Disable(u32, CollectionIntervalFactor),
}
