#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Subservice {
    TcEnableGeneration = 5,
    TcDisableGeneration = 6,
    TmHkPacket = 25,
    TcGenerateOneShotHk = 27,
    TcModifyCollectionInterval = 31,
}
