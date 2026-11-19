//! Counts and memory figures the scheduler reports about itself.

use super::process_slot::ProcessSlot;
use super::{Scheduler, SharedState, lock_or_recover};
use crate::distribution::Node;
use crate::process::ProcessStatus;

impl SharedState {
    /// Return the number of alive processes tracked by the scheduler.
    #[must_use]
    pub(super) fn process_count(&self) -> usize {
        self.process_table.len()
    }

    /// Return the configured number of normal scheduler threads.
    #[must_use]
    pub(super) fn scheduler_count(&self) -> usize {
        self.thread_count
    }

    /// Return the current number of interned atoms.
    #[must_use]
    pub(super) fn atom_count(&self) -> usize {
        self.atom_table.len()
    }

    /// Return an approximate memory summary for OTP compatibility probes.
    #[must_use]
    pub(super) fn memory_summary(&self) -> crate::native::system_info_bifs::MemorySummary {
        let (process_heap_words, binary) = self.process_heap_and_binary_words();

        let processes =
            process_heap_words.saturating_mul(crate::native::system_info_bifs::WORDSIZE_BYTES);
        let atom = self
            .atom_count()
            .saturating_mul(crate::native::system_info_bifs::WORDSIZE_BYTES);
        crate::native::system_info_bifs::MemorySummary::from_components(processes, atom, binary)
    }

    /// Return approximate process heap and virtual binary memory words.
    #[must_use]
    pub(super) fn process_heap_and_binary_words(&self) -> (usize, usize) {
        let mut process_heap_words = 0usize;
        let mut binary = 0usize;

        for entry in &self.process_bodies {
            match &*lock_or_recover(&entry) {
                ProcessSlot::Present(scheduled) => {
                    if matches!(scheduled.0.status(), ProcessStatus::Exited(_)) {
                        continue;
                    }
                    process_heap_words =
                        process_heap_words.saturating_add(scheduled.0.heap().total_used());
                    binary = binary.saturating_add(scheduled.0.virtual_binary_heap());
                }
                ProcessSlot::Executing(metadata) => {
                    process_heap_words = process_heap_words.saturating_add(metadata.heap_size);
                    binary = binary.saturating_add(metadata.binary_heap_size);
                }
                ProcessSlot::Absent => {}
            }
        }

        (process_heap_words, binary)
    }
}

impl Scheduler {
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.shared.scheduler_count()
    }
    #[must_use]
    pub fn process_count(&self) -> usize {
        self.shared.process_count()
    }
    #[must_use]
    pub fn scheduler_count(&self) -> usize {
        self.shared.scheduler_count()
    }
    #[must_use]
    pub fn atom_count(&self) -> usize {
        self.shared.atom_count()
    }
    #[must_use]
    pub fn atom_limit(&self) -> usize {
        self.shared.atom_table.limit()
    }
    #[must_use]
    pub fn local_node(&self) -> Node {
        self.shared.local_node
    }
    #[must_use]
    pub fn worker_names(&self) -> &[String] {
        &self.worker_names
    }
}
