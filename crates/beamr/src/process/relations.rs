//! A process's links and monitors, held so that a process watching or linked
//! to many others stays cheap.
//!
//! Two costs grew with the number of relations before. Looking one up was a
//! scan of a list, so adding n of them cost n². And every time a process ran,
//! the scheduler copied all of them into the process's slot and merged them
//! back afterwards. Liminal's channel supervisor watches every channel actor;
//! at 17,000 actors that merge kept a scheduler thread busy without end.
//!
//! Here each list carries an index, so a lookup is constant time, and each is
//! held behind an [`Arc`]: the slot shares the running process's copy and
//! only pays for a copy if one side changes it while the process runs.

// Without std there is no hashing collection, so a build without it keeps
// the same lists in B-trees: logarithmic rather than constant, still never a
// scan. Guarded as `process/mod.rs` guards its own `alloc` import, because
// lib.rs's `extern crate alloc` is itself conditional.
#[cfg(feature = "std")]
use Arc;
#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeMap as Map, BTreeSet as Set};
#[cfg(not(feature = "std"))]
use alloc::sync::Arc;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
#[cfg(feature = "std")]
use std::collections::{HashMap as Map, HashSet as Set};

use core::hash::Hash;

use super::{Monitor, RemotePid};

/// What a link list can hold: hashable for std's sets, ordered for the
/// B-trees used without std.
pub(crate) trait LinkId: Copy + Eq + Hash + Ord {}

impl<T: Copy + Eq + Hash + Ord> LinkId for T {}

/// Distinct ids in the order they were added, with constant-time lookup.
///
/// Order is kept because exit signals go to links in the order they were
/// made, which replays rely on.
#[derive(Clone, Debug)]
pub(crate) struct LinkList<T> {
    order: Vec<T>,
    held: Set<T>,
}

impl<T> Default for LinkList<T> {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            held: Set::new(),
        }
    }
}

impl<T: LinkId> LinkList<T> {
    /// The ids, oldest first.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.order
    }

    /// Whether `id` is held.
    #[must_use]
    pub fn contains(&self, id: &T) -> bool {
        self.held.contains(id)
    }

    /// Add `id` at the end; `false` if it was already held.
    pub fn insert(&mut self, id: T) -> bool {
        if !self.held.insert(id) {
            return false;
        }
        self.order.push(id);
        true
    }

    /// Remove `id`, keeping the order of the rest; `false` if it was not held.
    pub fn remove(&mut self, id: &T) -> bool {
        if !self.held.remove(id) {
            return false;
        }
        if let Some(index) = self.order.iter().position(|held| held == id) {
            self.order.remove(index);
        }
        true
    }

    /// Every id, oldest first, leaving the list empty.
    pub fn take(&mut self) -> Vec<T> {
        self.held.clear();
        core::mem::take(&mut self.order)
    }

    /// How many ids are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether no id is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// A process's monitors, both those it holds as watcher and those that
/// watch it, with constant-time add and remove by reference.
///
/// A process that monitors itself holds the same monitor twice, once from
/// each side, so a reference may appear more than once. Removing a reference
/// removes one copy. Order is not kept: removal moves the last monitor into
/// the gap, and nothing reads monitors in order.
#[derive(Clone, Debug, Default)]
pub(crate) struct MonitorTable {
    monitors: Vec<Monitor>,
    /// Where each reference held exactly once sits in `monitors`.
    at: Map<u64, usize>,
    /// References held more than once, with how many copies.
    repeated: Map<u64, usize>,
}

impl MonitorTable {
    /// The monitors, in no particular order.
    #[must_use]
    pub const fn as_vec(&self) -> &Vec<Monitor> {
        &self.monitors
    }

    /// Whether a monitor with `reference` is held.
    #[must_use]
    pub fn contains(&self, reference: u64) -> bool {
        self.at.contains_key(&reference) || self.repeated.contains_key(&reference)
    }

    /// Add `monitor`.
    pub fn insert(&mut self, monitor: Monitor) {
        let reference = monitor.reference();
        if let Some(copies) = self.repeated.get_mut(&reference) {
            *copies += 1;
        } else if self.at.remove(&reference).is_some() {
            self.repeated.insert(reference, 2);
        } else {
            self.at.insert(reference, self.monitors.len());
        }
        self.monitors.push(monitor);
    }

    /// Remove one monitor with `reference`, if any is held.
    pub fn remove(&mut self, reference: u64) -> Option<Monitor> {
        let mut now_single = false;
        let index = if let Some(copies) = self.repeated.get_mut(&reference) {
            *copies -= 1;
            if *copies == 1 {
                self.repeated.remove(&reference);
                now_single = true;
            }
            self.position(reference)?
        } else {
            self.at.remove(&reference)?
        };
        let removed = self.monitors.swap_remove(index);
        if let Some(moved) = self.monitors.get(index)
            && let Some(slot) = self.at.get_mut(&moved.reference())
        {
            *slot = index;
        }
        if now_single && let Some(left) = self.position(reference) {
            // The last copy of a repeated reference is now held once.
            self.at.insert(reference, left);
        }
        Some(removed)
    }

    /// How many monitors are held, counting repeats.
    #[must_use]
    pub fn len(&self) -> usize {
        self.monitors.len()
    }

    /// Whether no monitor is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }

    /// Only for repeated references, which only a self-monitor makes.
    fn position(&self, reference: u64) -> Option<usize> {
        self.monitors
            .iter()
            .position(|monitor| monitor.reference() == reference)
    }
}

#[cfg(test)]
mod tests {
    use super::{LinkList, MonitorTable};
    use crate::process::Monitor;

    #[test]
    fn link_list_keeps_order_and_refuses_repeats() {
        let mut links = LinkList::default();
        assert!(links.insert(11));
        assert!(links.insert(13));
        assert!(!links.insert(11));
        assert!(links.insert(17));
        assert!(links.remove(&13));
        assert!(!links.remove(&13));
        assert_eq!(links.as_slice(), &[11, 17]);
        assert!(links.contains(&17));
        assert_eq!(links.take(), vec![11, 17]);
        assert!(links.is_empty() && !links.contains(&11));
    }

    #[test]
    fn monitor_table_removes_by_reference_and_keeps_its_index_right() {
        let mut table = MonitorTable::default();
        for reference in 1..=5 {
            table.insert(Monitor::new(reference, 7, 100 + reference));
        }
        assert_eq!(table.remove(2).map(Monitor::reference), Some(2));
        assert_eq!(table.remove(2), None);
        // 5 moved into 2's place; it must still be found there.
        assert_eq!(table.remove(5).map(Monitor::reference), Some(5));
        let mut left: Vec<u64> = table.as_vec().iter().map(|m| m.reference()).collect();
        left.sort_unstable();
        assert_eq!(left, vec![1, 3, 4]);
        assert!(table.contains(4) && !table.contains(5));
    }

    #[test]
    fn a_self_monitor_is_held_twice_and_removed_one_copy_at_a_time() {
        let mut table = MonitorTable::default();
        table.insert(Monitor::new(9, 7, 7));
        table.insert(Monitor::new(3, 7, 8));
        table.insert(Monitor::new(9, 7, 7));
        assert_eq!(table.len(), 3);
        assert!(table.remove(9).is_some());
        assert!(table.contains(9));
        assert!(table.remove(9).is_some());
        assert!(!table.contains(9));
        assert_eq!(table.remove(9), None);
        assert!(table.remove(3).is_some());
        assert!(table.is_empty());
    }

    #[test]
    fn a_watcher_of_many_adds_and_removes_in_linear_time() {
        // One monitor per watched process, as Liminal's channel supervisor
        // holds for every channel actor. A scan per lookup made this 10^10.
        const WATCHED: u64 = 100_000;
        let started = std::time::Instant::now();
        let mut table = MonitorTable::default();
        let mut links = LinkList::default();
        for reference in 0..WATCHED {
            table.insert(Monitor::new(reference, 7, reference));
            links.insert(reference);
        }
        for reference in (0..WATCHED).step_by(2) {
            assert!(table.remove(reference).is_some());
            assert!(links.contains(&reference));
        }
        let took = started.elapsed();
        assert_eq!(table.len(), usize::try_from(WATCHED / 2).unwrap_or(0));
        assert!(
            took < std::time::Duration::from_secs(2),
            "{WATCHED} monitors took {took:?}"
        );
    }
}

/// A process's links and monitors. Each is shared with the process's slot
/// while it runs, so changing one here copies it only if the slot still
/// holds the same list.
impl super::Process {
    /// Linked process IDs, oldest first.
    #[must_use]
    pub fn links(&self) -> &[u64] {
        self.links.as_slice()
    }

    /// Add a linked process id. Returns whether the ordered set changed.
    ///
    /// Link insertion order is preserved for deterministic exit propagation.
    /// Self-links and duplicate links are ignored.
    pub fn add_link(&mut self, pid: u64) -> bool {
        if pid == self.pid || self.links.contains(&pid) {
            return false;
        }
        Arc::make_mut(&mut self.links).insert(pid)
    }

    /// Remove a linked process id. Returns whether the ordered set changed.
    pub fn remove_link(&mut self, pid: u64) -> bool {
        if !self.links.contains(&pid) {
            return false;
        }
        Arc::make_mut(&mut self.links).remove(&pid)
    }

    /// Remove all links and return the previous link set in insertion order.
    pub fn take_links(&mut self) -> Vec<u64> {
        take_list(&mut self.links)
    }

    /// Linked remote process IDs, oldest first.
    #[must_use]
    pub fn remote_links(&self) -> &[RemotePid] {
        self.remote_links.as_slice()
    }

    /// Add a linked remote process id. Returns whether the ordered set changed.
    ///
    /// Link insertion order is preserved separately from local links so remote
    /// exit propagation is deterministic without losing node/serial identity.
    pub fn add_remote_link(&mut self, pid: RemotePid) -> bool {
        if self.remote_links.contains(&pid) {
            return false;
        }
        Arc::make_mut(&mut self.remote_links).insert(pid)
    }

    /// Remove a linked remote process id. Returns whether the ordered set changed.
    pub fn remove_remote_link(&mut self, pid: RemotePid) -> bool {
        if !self.remote_links.contains(&pid) {
            return false;
        }
        Arc::make_mut(&mut self.remote_links).remove(&pid)
    }

    /// Remove all remote links and return the previous set in insertion order.
    pub fn take_remote_links(&mut self) -> Vec<RemotePid> {
        take_list(&mut self.remote_links)
    }

    /// Monitor metadata attached to this process, in no particular order.
    #[must_use]
    pub fn monitors(&self) -> &Vec<Monitor> {
        self.monitors.as_vec()
    }

    /// Add monitor metadata owned by or targeting this process.
    pub fn add_monitor(&mut self, monitor: Monitor) {
        Arc::make_mut(&mut self.monitors).insert(monitor);
    }

    /// Remove monitor metadata by reference. Returns removed metadata.
    pub fn remove_monitor(&mut self, reference: u64) -> Option<Monitor> {
        if !self.monitors.contains(reference) {
            return None;
        }
        Arc::make_mut(&mut self.monitors).remove(reference)
    }

    /// The shared lists, for the scheduler's slot while this process runs.
    pub(crate) fn shared_relations(&self) -> SharedRelations {
        SharedRelations {
            links: Arc::clone(&self.links),
            remote_links: Arc::clone(&self.remote_links),
            monitors: Arc::clone(&self.monitors),
        }
    }

    /// Take back the slot's lists after a run. The slot's monitors and
    /// remote links are authoritative: every change made while the process
    /// ran was made there. Local links are the union of both sides, oldest
    /// of the process's first, because the process may link while it runs.
    pub(crate) fn restore_relations(&mut self, slot: SharedRelations) {
        self.monitors = slot.monitors;
        self.remote_links = slot.remote_links;
        if !Arc::ptr_eq(&self.links, &slot.links) {
            let pid = self.pid;
            let mine = Arc::make_mut(&mut self.links);
            for &linked in slot.links.as_slice() {
                if linked != pid {
                    mine.insert(linked);
                }
            }
        }
    }
}

/// A running process's links and monitors as its slot holds them.
#[derive(Clone, Debug, Default)]
pub(crate) struct SharedRelations {
    pub(crate) links: Arc<LinkList<u64>>,
    pub(crate) remote_links: Arc<LinkList<super::RemotePid>>,
    pub(crate) monitors: Arc<MonitorTable>,
}

fn take_list<T: LinkId>(list: &mut Arc<LinkList<T>>) -> Vec<T> {
    match Arc::try_unwrap(core::mem::take(list)) {
        Ok(mut owned) => owned.take(),
        Err(shared) => shared.as_slice().to_vec(),
    }
}
