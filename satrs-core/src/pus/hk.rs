#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Subservice {
    EnableGeneration = 5,
    DisableGeneration = 6,
    HkPacket = 25,
    GenerateOneShotHk = 27,
    ModifyCollectionInterval = 31,
}
