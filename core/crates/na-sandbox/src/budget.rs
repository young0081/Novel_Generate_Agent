//! Resource budgets for a single tool call or agent step.
//!
//! Tools and the agent loop run under hard ceilings so a runaway model cannot
//! exhaust the machine: a maximum amount of captured output, a wall-clock
//! deadline, and a maximum number of steps. [`ResourceBudget`] bundles those
//! limits; [`StepCounter`] tracks consumed steps. Exceeding any limit yields a
//! [`CoreError::budget`] (kind [`BudgetExceeded`](na_common::ErrorKind::BudgetExceeded)).

use std::time::Duration;

use na_common::{CoreError, Result};

/// Hard ceilings applied to a unit of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceBudget {
    /// Maximum number of output bytes we will buffer/return.
    pub max_output_bytes: usize,
    /// Wall-clock deadline in milliseconds.
    pub max_wall_ms: u64,
    /// Maximum number of discrete steps (e.g. agent-loop iterations).
    pub max_steps: u32,
}

impl ResourceBudget {
    /// Construct an explicit budget.
    pub fn new(max_output_bytes: usize, max_wall_ms: u64, max_steps: u32) -> Self {
        ResourceBudget {
            max_output_bytes,
            max_wall_ms,
            max_steps,
        }
    }

    /// The wall-clock limit as a [`Duration`].
    pub fn wall_duration(&self) -> Duration {
        Duration::from_millis(self.max_wall_ms)
    }

    /// Error unless `n` bytes fit within [`max_output_bytes`](Self::max_output_bytes).
    ///
    /// Use this when you are about to accept/return a buffer of `n` bytes.
    pub fn check_bytes(&self, n: usize) -> Result<()> {
        if n > self.max_output_bytes {
            Err(CoreError::budget(format!(
                "output of {n} bytes exceeds budget of {} bytes",
                self.max_output_bytes
            )))
        } else {
            Ok(())
        }
    }

    /// Like [`check_bytes`](Self::check_bytes) but for an incremental writer:
    /// given the bytes already accumulated and the number about to be added,
    /// error if the new total would overflow the budget. Saturating addition
    /// means a `usize` overflow is itself reported as a budget breach rather
    /// than wrapping.
    pub fn check_additional_bytes(&self, already: usize, adding: usize) -> Result<()> {
        let total = already.saturating_add(adding);
        self.check_bytes(total)
    }

    /// Allocate a fresh [`StepCounter`] bounded by [`max_steps`](Self::max_steps).
    pub fn step_counter(&self) -> StepCounter {
        StepCounter::new(self.max_steps)
    }
}

impl Default for ResourceBudget {
    /// Sane defaults: 64 KiB of output, 30 s wall-clock, 50 steps.
    fn default() -> Self {
        ResourceBudget {
            max_output_bytes: 64 * 1024,
            max_wall_ms: 30_000,
            max_steps: 50,
        }
    }
}

/// A monotonic step counter that refuses to exceed its maximum.
///
/// Call [`tick`](StepCounter::tick) once per step; it increments the count and
/// returns `Err(BudgetExceeded)` the moment the count would pass `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepCounter {
    /// Steps consumed so far.
    pub used: u32,
    /// Maximum allowed steps.
    pub max: u32,
}

impl StepCounter {
    /// Create a counter that permits up to `max` ticks.
    pub fn new(max: u32) -> Self {
        StepCounter { used: 0, max }
    }

    /// Record one step.
    ///
    /// Returns `Ok(())` for the 1st..=`max`th call, and
    /// `Err(BudgetExceeded)` once the budget is spent (including every call
    /// after the limit). A `max` of `0` rejects the very first tick.
    pub fn tick(&mut self) -> Result<()> {
        if self.used >= self.max {
            return Err(CoreError::budget(format!(
                "step budget exhausted ({} of {} steps used)",
                self.used, self.max
            )));
        }
        self.used += 1;
        Ok(())
    }

    /// Steps still available before the budget is exhausted.
    pub fn remaining(&self) -> u32 {
        self.max.saturating_sub(self.used)
    }

    /// Whether the next [`tick`](Self::tick) would fail.
    pub fn is_exhausted(&self) -> bool {
        self.used >= self.max
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let b = ResourceBudget::default();
        assert_eq!(b.max_output_bytes, 64 * 1024);
        assert_eq!(b.max_wall_ms, 30_000);
        assert_eq!(b.max_steps, 50);
        assert_eq!(b.wall_duration(), Duration::from_millis(30_000));
    }

    #[test]
    fn check_bytes_boundary() {
        let b = ResourceBudget::new(100, 1000, 5);
        assert!(b.check_bytes(0).is_ok());
        assert!(b.check_bytes(100).is_ok()); // exactly at the limit is fine
        let err = b.check_bytes(101).unwrap_err();
        assert!(err.is(na_common::ErrorKind::BudgetExceeded), "{err}");
    }

    #[test]
    fn check_additional_bytes_overflow() {
        let b = ResourceBudget::new(100, 1000, 5);
        assert!(b.check_additional_bytes(60, 40).is_ok()); // 100 total
        let err = b.check_additional_bytes(60, 41).unwrap_err(); // 101 total
        assert!(err.is(na_common::ErrorKind::BudgetExceeded));
        // usize overflow is reported, not wrapped.
        let err2 = b.check_additional_bytes(usize::MAX, 1).unwrap_err();
        assert!(err2.is(na_common::ErrorKind::BudgetExceeded));
    }

    #[test]
    fn step_counter_counts_and_stops() {
        let mut c = StepCounter::new(3);
        assert_eq!(c.remaining(), 3);
        assert!(c.tick().is_ok()); // 1
        assert!(c.tick().is_ok()); // 2
        assert!(c.tick().is_ok()); // 3
        assert_eq!(c.remaining(), 0);
        assert!(c.is_exhausted());
        let err = c.tick().unwrap_err(); // 4th -> over budget
        assert!(err.is(na_common::ErrorKind::BudgetExceeded), "{err}");
        // Stays errored.
        assert!(c.tick().is_err());
        assert_eq!(c.used, 3); // used never exceeds max
    }

    #[test]
    fn zero_step_budget_rejects_immediately() {
        let mut c = StepCounter::new(0);
        assert!(c.is_exhausted());
        assert!(c.tick().is_err());
        assert_eq!(c.remaining(), 0);
    }

    #[test]
    fn budget_builds_counter() {
        let b = ResourceBudget::new(10, 10, 2);
        let mut c = b.step_counter();
        assert_eq!(c.max, 2);
        assert!(c.tick().is_ok());
        assert!(c.tick().is_ok());
        assert!(c.tick().is_err());
    }

    #[test]
    fn budget_is_copy() {
        let b = ResourceBudget::default();
        let b2 = b; // Copy
        assert_eq!(b, b2);
    }
}
