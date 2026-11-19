//! How a process is scheduled: its priority, and the reduction budget of
//! its current time slice.

use super::{Priority, Process};

impl Process {
    /// Current scheduling priority.
    #[must_use]
    pub const fn priority(&self) -> Priority {
        self.priority
    }

    /// Set this process's scheduling priority.
    pub const fn set_priority(&mut self, priority: Priority) {
        self.priority = priority;
    }

    /// Current reduction budget remainder.
    #[must_use]
    pub const fn reduction_counter(&self) -> u32 {
        self.reduction_counter
    }

    /// Subtract reductions from the current budget, saturating at zero.
    pub fn decrement_reductions(&mut self, n: u32) {
        self.reduction_counter = self.reduction_counter.saturating_sub(n);
    }

    /// Returns true when no reductions remain in this time slice.
    #[must_use]
    pub const fn reductions_exhausted(&self) -> bool {
        self.reduction_counter == 0
    }

    /// Reset the reduction counter for a new scheduler time slice.
    pub const fn reset_reductions(&mut self, budget: u32) {
        self.reduction_counter = budget;
    }
}
