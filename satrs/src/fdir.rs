//! # FDIR (Fault Detection, Isolation and Recovery) helpers
//!
//! A fault counter tracks a monotonic fault count, decrements it over time when faults stop
//! occurring, and reports when a configured failure threshold has been exceeded. This is the
//! typical building block used to turn a stream of transient error reports into a single
//! "component is faulty" decision without reacting to the first isolated error.
//!
//! The design follows the FSFW `FaultCounter`:
//! <https://egit.irs.uni-stuttgart.de/KSat/fsfw/src/branch/main/src/fsfw/fdir/FaultCounter.h>
//!
//! Pick a variant based on what clock is available:
//!
//! - [FaultCounterStd]: `std::time::Instant`, behind the `std` feature.
#![cfg_attr(
    feature = "embassy-time",
    doc = "- [FaultCounterEmbassy]: `embassy_time::Instant`, behind the `embassy-time` feature."
)]
#![deny(missing_docs)]

/// Fault counter backed by [std::time::Instant].
#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub struct FaultCounterStd {
    fault_count: u32,
    failure_threshold: u32,
    decrement_after: core::time::Duration,
    last_decrement: Option<std::time::Instant>,
}

#[cfg(feature = "std")]
impl FaultCounterStd {
    /// Create a new [`FaultCounterStd`].
    ///
    /// - `failure_threshold`: threshold above which [`Self::above_threshold`] returns `true` and
    ///   resets the internal count.
    /// - `decrement_after`: minimum duration between automatic decrements performed by
    ///   [`Self::try_decrement`].
    pub fn new(failure_threshold: u32, decrement_after: core::time::Duration) -> Self {
        Self {
            fault_count: 0,
            failure_threshold,
            decrement_after,
            last_decrement: None,
        }
    }

    /// Current fault count.
    pub fn fault_count(&self) -> u32 {
        self.fault_count
    }

    /// Increase the fault count by `1`.
    ///
    /// If the counter was previously `0`, this starts a new decrement clock.
    pub fn increment(&mut self) {
        if self.fault_count == 0 {
            self.last_decrement = Some(std::time::Instant::now());
        }
        self.fault_count += 1;
    }

    /// Increase the fault count by `n`.
    pub fn increment_n(&mut self, n: u32) {
        for _ in 0..n {
            self.increment();
        }
    }

    fn has_decrement_timedout(&self) -> bool {
        match self.last_decrement {
            Some(last_decrement) => last_decrement.elapsed() >= self.decrement_after,
            None => false,
        }
    }

    /// Decrease the fault count by `1` if the decrement timeout elapsed.
    ///
    /// Returns `true` if a decrement was performed, `false` otherwise. A decrement is only
    /// performed when the counter is non-zero and at least `decrement_after` has elapsed since
    /// the last decrement.
    pub fn try_decrement(&mut self) -> bool {
        if self.fault_count == 0 || !self.has_decrement_timedout() {
            return false;
        }
        self.last_decrement = Some(std::time::Instant::now());
        self.fault_count -= 1;
        true
    }

    /// Check whether the counter exceeded the failure threshold.
    ///
    /// Returns `true` when `fault_count > failure_threshold`. In that case, the counter is reset
    /// to `0`.
    pub fn above_threshold(&mut self) -> bool {
        if self.fault_count > self.failure_threshold {
            self.fault_count = 0;
            return true;
        }
        false
    }

    /// Convenience helper to increment once and immediately check the threshold.
    pub fn increment_and_check(&mut self) -> bool {
        self.increment();
        self.above_threshold()
    }

    /// Clear the counter and decrement timing state.
    pub fn clear(&mut self) {
        self.fault_count = 0;
        self.last_decrement = None;
    }

    /// Update the failure threshold used by [`Self::above_threshold`].
    pub fn set_failure_threshold(&mut self, threshold: u32) {
        self.failure_threshold = threshold;
    }

    /// Update the minimum interval between automatic decrements.
    pub fn set_decrement_after(&mut self, duration: core::time::Duration) {
        self.decrement_after = duration;
    }
}

/// Fault counter backed by [embassy_time::Instant].
#[cfg(feature = "embassy-time")]
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FaultCounterEmbassy {
    fault_count: u32,
    failure_threshold: u32,
    decrement_after: embassy_time::Duration,
    last_decrement: Option<embassy_time::Instant>,
}

#[cfg(feature = "embassy-time")]
impl FaultCounterEmbassy {
    /// Create a new [`FaultCounterEmbassy`].
    ///
    /// - `failure_threshold`: threshold above which [`Self::above_threshold`] returns `true` and
    ///   resets the internal count.
    /// - `decrement_after`: minimum duration between automatic decrements performed by
    ///   [`Self::try_decrement`].
    pub fn new(failure_threshold: u32, decrement_after: embassy_time::Duration) -> Self {
        Self {
            fault_count: 0,
            failure_threshold,
            decrement_after,
            last_decrement: None,
        }
    }

    /// Current fault count.
    pub fn fault_count(&self) -> u32 {
        self.fault_count
    }

    /// Increase the fault count by `1`.
    ///
    /// If the counter was previously `0`, this starts a new decrement clock.
    pub fn increment(&mut self) {
        if self.fault_count == 0 {
            self.last_decrement = Some(embassy_time::Instant::now());
        }
        self.fault_count += 1;
    }

    /// Increase the fault count by `n`.
    pub fn increment_n(&mut self, n: u32) {
        for _ in 0..n {
            self.increment();
        }
    }

    fn has_decrement_timedout(&self) -> bool {
        match self.last_decrement {
            Some(last_decrement) => {
                embassy_time::Instant::now().duration_since(last_decrement) >= self.decrement_after
            }
            None => false,
        }
    }

    /// Decrease the fault count by `1` if the decrement timeout elapsed.
    ///
    /// Returns `true` if a decrement was performed, `false` otherwise. A decrement is only
    /// performed when the counter is non-zero and at least `decrement_after` has elapsed since
    /// the last decrement.
    pub fn try_decrement(&mut self) -> bool {
        if self.fault_count == 0 || !self.has_decrement_timedout() {
            return false;
        }
        self.last_decrement = Some(embassy_time::Instant::now());
        self.fault_count -= 1;
        true
    }

    /// Check whether the counter exceeded the failure threshold.
    ///
    /// Returns `true` when `fault_count > failure_threshold`. In that case, the counter is reset
    /// to `0`.
    pub fn above_threshold(&mut self) -> bool {
        if self.fault_count > self.failure_threshold {
            self.fault_count = 0;
            return true;
        }
        false
    }

    /// Convenience helper to increment once and immediately check the threshold.
    pub fn increment_and_check(&mut self) -> bool {
        self.increment();
        self.above_threshold()
    }

    /// Clear the counter and decrement timing state.
    pub fn clear(&mut self) {
        self.fault_count = 0;
        self.last_decrement = None;
    }

    /// Update the failure threshold used by [`Self::above_threshold`].
    pub fn set_failure_threshold(&mut self, threshold: u32) {
        self.failure_threshold = threshold;
    }

    /// Update the minimum interval between automatic decrements.
    pub fn set_decrement_after(&mut self, duration: embassy_time::Duration) {
        self.decrement_after = duration;
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn threshold_not_exceeded_below_limit() {
        let mut fc = FaultCounterStd::new(2, Duration::from_secs(60));
        assert!(!fc.increment_and_check());
        assert!(!fc.increment_and_check());
        assert_eq!(fc.fault_count(), 2);
    }

    #[test]
    fn threshold_exceeded_resets_counter() {
        let mut fc = FaultCounterStd::new(2, Duration::from_secs(60));
        fc.increment_n(3);
        assert!(fc.above_threshold());
        assert_eq!(fc.fault_count(), 0);
        assert!(!fc.above_threshold());
    }

    #[test]
    fn decrement_only_after_timeout() {
        let mut fc = FaultCounterStd::new(5, Duration::from_millis(20));
        fc.increment();
        assert!(!fc.try_decrement());
        thread::sleep(Duration::from_millis(30));
        assert!(fc.try_decrement());
        assert_eq!(fc.fault_count(), 0);
    }

    #[test]
    fn decrement_noop_when_empty() {
        let mut fc = FaultCounterStd::new(5, Duration::from_millis(1));
        thread::sleep(Duration::from_millis(2));
        assert!(!fc.try_decrement());
    }

    #[test]
    fn increment_after_empty_resets_decrement_timing() {
        let mut fc = FaultCounterStd::new(5, Duration::from_millis(20));
        fc.increment();
        thread::sleep(Duration::from_millis(30));
        assert!(fc.try_decrement());
        // Counter is 0 again, incrementing should require a fresh decrement_after wait.
        fc.increment();
        assert!(!fc.try_decrement());
    }

    #[test]
    fn clear_resets_state() {
        let mut fc = FaultCounterStd::new(1, Duration::from_secs(60));
        fc.increment_n(2);
        fc.clear();
        assert_eq!(fc.fault_count(), 0);
        assert!(!fc.above_threshold());
    }
}
