//! The per-process logical clock deterministic replay orders events by.

use super::Process;

impl Process {
    /// Current per-process logical clock used by deterministic replay.
    #[must_use]
    pub const fn logical_clock(&self) -> u64 {
        self.logical_clock
    }

    /// Advance the logical clock for a local causal event.
    pub fn tick_logical_clock(&mut self) -> u64 {
        self.logical_clock = self.logical_clock.saturating_add(1);
        self.logical_clock
    }

    /// Merge a sender clock into this process and advance for message delivery.
    pub fn observe_message_clock(&mut self, sender_clock: u64) -> u64 {
        self.logical_clock = self.logical_clock.max(sender_clock).saturating_add(1);
        self.logical_clock
    }

    /// Set the logical clock from a recorded replay delivery.
    pub const fn set_logical_clock(&mut self, clock: u64) {
        self.logical_clock = clock;
    }
}
