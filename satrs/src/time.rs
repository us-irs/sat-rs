use core::fmt::Debug;

/// Generic abstraction for a check/countdown timer.
pub trait CountdownProvider: Debug {
    fn has_expired(&self) -> bool;
    fn reset(&mut self);
}
