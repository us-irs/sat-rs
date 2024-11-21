use core::fmt::Debug;

pub trait SpiInterface {
    type Error: Debug;
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<(), Self::Error>;
}
