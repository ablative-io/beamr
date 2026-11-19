//! Publishing a process's exit: the single write path for its exit outcome
//! and its legacy tombstone.

use super::{SharedState, exit_capture};
use crate::ets::copy::OwnedTerm;
use crate::process::ExitReason;

impl SharedState {
    /// Publish an exit outcome and insert its bounded legacy tombstone.
    ///
    /// This is the single write path. A private owned copy of the result is
    /// installed before the takeable event, establishing the exit-event
    /// happens-before guarantee without changing any legacy consuming store.
    /// Legacy tombstone eviction continues to remove all legacy satellites;
    /// only the additive outcome copy is retained until taken.
    pub(super) fn insert_exit_tombstone(&self, pid: u64, reason: ExitReason) {
        use dashmap::mapref::entry::Entry;

        match self.exit_results.entry(pid) {
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                entry.insert(OwnedTerm::immediate(crate::term::Term::NIL));
            }
        }
        let outcome = self
            .exit_results
            .get(&pid)
            .map(|result| exit_capture::capture_term(result.root()))
            .unwrap_or_else(|| OwnedTerm::immediate(crate::term::Term::NIL));
        if let Some(evicted) = self.exit_tombstones.insert_outcome(pid, reason, outcome) {
            self.exit_results.remove(&evicted);
            self.exit_errors.remove(&evicted);
            self.exit_exceptions.remove(&evicted);
        }
    }
}
