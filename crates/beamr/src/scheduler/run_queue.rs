//! Per-thread priority run queues backed by lock-free work-stealing deques.
//!
//! Owner pops and steals are both FIFO: within a priority lane, co-resident
//! runnable processes round-robin. The owner pop MUST NOT be LIFO — a process
//! that requeues itself every slice (a busy-polling native handler returning
//! `NativeOutcome::Continue`) would be re-popped immediately after its own
//! push, starving every other pid on that thread's queue indefinitely: a
//! starved pid is runnable (not in the wait set), so message-delivery wakes
//! no-op on it, and before B-185 a single pid queued behind an owner in a
//! slice was never stolen. Process IDs are queued rather than
//! process bodies because `Process` is intentionally `!Send`.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_deque::{Steal, Stealer, Worker};

use crate::process::Priority;

/// Default number of non-low owner pops allowed before an available low-priority
/// process is preferred to guarantee bounded low-priority progress.
pub const DEFAULT_LOW_PRIORITY_FAIRNESS_WINDOW: usize = 8;

/// Stealer handles for each priority lane of a scheduler run queue.
#[derive(Clone)]
pub struct PriorityStealers {
    max: Stealer<u64>,
    high: Stealer<u64>,
    normal: Stealer<u64>,
    low: Stealer<u64>,
    /// The owner's in-slice flag (B-185 R1), read by a thief as it steals.
    owner_in_slice: Arc<AtomicBool>,
}

/// A per-thread priority run queue that stores process IDs.
pub struct RunQueue {
    max: Worker<u64>,
    high: Worker<u64>,
    normal: Worker<u64>,
    low: Worker<u64>,
    low_fairness_window: usize,
    non_low_pops_since_low: Cell<usize>,
    /// Raised (SeqCst) while the owner runs a process slice (B-185 R1).
    in_slice: Arc<AtomicBool>,
}

impl RunQueue {
    /// Create a new empty run queue.
    #[must_use]
    pub fn new() -> Self {
        Self::with_low_fairness_window(DEFAULT_LOW_PRIORITY_FAIRNESS_WINDOW)
    }

    /// Create a new empty run queue with a configurable low-priority fairness window.
    #[must_use]
    pub fn with_low_fairness_window(low_fairness_window: usize) -> Self {
        Self {
            max: Worker::new_fifo(),
            high: Worker::new_fifo(),
            normal: Worker::new_fifo(),
            low: Worker::new_fifo(),
            low_fairness_window,
            non_low_pops_since_low: Cell::new(0),
            in_slice: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Push a process ID onto the owner side of the normal-priority queue.
    pub fn push(&self, pid: u64) {
        self.push_with_priority(pid, Priority::Normal);
    }

    /// Push a process ID onto the owner side of the matching priority queue.
    pub fn push_with_priority(&self, pid: u64, priority: Priority) {
        self.worker(priority).push(pid);
    }

    /// Pop a process ID from the owner side of the highest eligible priority queue.
    #[must_use]
    pub fn pop(&self) -> Option<u64> {
        if self.should_prefer_low()
            && let Some(pid) = self.pop_priority(Priority::Low)
        {
            self.non_low_pops_since_low.set(0);
            return Some(pid);
        }

        for priority in [
            Priority::Max,
            Priority::High,
            Priority::Normal,
            Priority::Low,
        ] {
            if let Some(pid) = self.pop_priority(priority) {
                self.record_pop(priority);
                return Some(pid);
            }
        }
        None
    }

    /// Approximate number of queued process IDs across all priority queues.
    #[must_use]
    pub fn len(&self) -> usize {
        self.max.len() + self.high.len() + self.normal.len() + self.low.len()
    }

    /// Whether this queue is currently empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.max.is_empty() && self.high.is_empty() && self.normal.is_empty() && self.low.is_empty()
    }

    /// Publish that this queue's owner is about to run a process slice
    /// (B-185 R1, §3.8.2 C5): the flag is raised `SeqCst` here, before the
    /// slice starts, and lowered when the returned guard drops, so it is
    /// lowered on every way out of the slice, an unwinding panic included.
    #[must_use = "the flag is lowered when the guard drops"]
    pub(crate) fn enter_slice(&self) -> InSlice<'_> {
        self.in_slice.store(true, Ordering::SeqCst);
        InSlice {
            flag: &self.in_slice,
        }
    }

    /// Create stealer handles for other scheduler threads.
    #[must_use]
    pub fn stealer(&self) -> PriorityStealers {
        PriorityStealers {
            max: self.max.stealer(),
            high: self.high.stealer(),
            normal: self.normal.stealer(),
            low: self.low.stealer(),
            owner_in_slice: Arc::clone(&self.in_slice),
        }
    }

    /// Steal approximately half the items from `victim` into this queue.
    ///
    /// An empty queue gives nothing. A queue of one is left to its owner when
    /// the owner is between slices, since the owner is about to pop it; when
    /// the owner is running a slice (B-185 R1, §3.8.2 C5) that one process is
    /// taken, because it would otherwise wait beside an idle worker for the
    /// slice to end, however long or blocking the slice is. From two or more,
    /// half is taken, rounded down, whatever the owner is doing. Priority lanes
    /// are stolen independently; victim selection remains the caller's
    /// responsibility.
    pub fn steal_half_from(&self, victim: &PriorityStealers) -> usize {
        let victim_len = victim.len();
        if victim_len == 0 {
            return 0;
        }
        // The thief's half of C5's Dekker pair: its idle-window raise and
        // fence (`enter_idle_window`) came before this read of the flag.
        let owner_in_slice = victim.owner_in_slice.load(Ordering::SeqCst);
        let mut remaining = if owner_in_slice {
            (victim_len / 2).max(1)
        } else {
            victim_len / 2
        };
        if remaining == 0 {
            return 0;
        }

        let mut stolen = 0;
        for priority in [
            Priority::Max,
            Priority::High,
            Priority::Normal,
            Priority::Low,
        ] {
            if remaining == 0 {
                break;
            }
            let count = self.steal_from_priority(victim, priority, remaining);
            stolen += count;
            remaining = remaining.saturating_sub(count);
        }
        stolen
    }

    fn worker(&self, priority: Priority) -> &Worker<u64> {
        match priority {
            Priority::Low => &self.low,
            Priority::Normal => &self.normal,
            Priority::High => &self.high,
            Priority::Max => &self.max,
        }
    }

    fn pop_priority(&self, priority: Priority) -> Option<u64> {
        self.worker(priority).pop()
    }

    fn should_prefer_low(&self) -> bool {
        self.low_fairness_window > 0
            && self.non_low_pops_since_low.get() >= self.low_fairness_window
            && !self.low.is_empty()
    }

    fn record_pop(&self, priority: Priority) {
        if priority == Priority::Low {
            self.non_low_pops_since_low.set(0);
        } else {
            self.non_low_pops_since_low
                .set(self.non_low_pops_since_low.get().saturating_add(1));
        }
    }

    fn steal_from_priority(
        &self,
        victim: &PriorityStealers,
        priority: Priority,
        limit: usize,
    ) -> usize {
        let stealer = victim.stealer(priority);
        let before = self.worker(priority).len();
        match stealer.steal_batch_with_limit_and_pop(self.worker(priority), limit) {
            Steal::Success(pid) => {
                self.worker(priority).push(pid);
                self.worker(priority).len().saturating_sub(before)
            }
            Steal::Empty | Steal::Retry => 0,
        }
    }
}

impl PriorityStealers {
    fn len(&self) -> usize {
        self.max.len() + self.high.len() + self.normal.len() + self.low.len()
    }

    /// Processes queued behind this stealer, for B-181's loss detector
    /// (test builds only).
    #[cfg(test)]
    pub(in crate::scheduler) fn queued(&self) -> usize {
        self.len()
    }

    fn stealer(&self, priority: Priority) -> &Stealer<u64> {
        match priority {
            Priority::Low => &self.low,
            Priority::Normal => &self.normal,
            Priority::High => &self.high,
            Priority::Max => &self.max,
        }
    }
}

/// The owner's in-slice flag, raised by [`RunQueue::enter_slice`] and lowered
/// (`SeqCst`) when this guard drops (B-185 R1).
pub(crate) struct InSlice<'queue> {
    flag: &'queue AtomicBool,
}

impl Drop for InSlice<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

impl Default for RunQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::RunQueue;
    use crate::process::Priority;

    #[test]
    fn push_then_pop_returns_same_process() {
        let queue = RunQueue::new();
        queue.push(42);

        assert_eq!(queue.pop(), Some(42));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn owner_pop_is_fifo_within_priority() {
        // FIFO is load-bearing: a self-requeueing (busy-poll) process must go
        // to the back so co-resident runnable processes round-robin instead
        // of starving (see the module docs).
        let queue = RunQueue::new();
        queue.push(1);
        queue.push(2);
        queue.push(3);

        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
    }

    #[test]
    fn max_priority_dequeues_before_normal() {
        let queue = RunQueue::new();
        queue.push_with_priority(1, Priority::Normal);
        queue.push_with_priority(2, Priority::Max);

        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(1));
    }

    #[test]
    fn low_priority_progresses_under_high_pressure() {
        let queue = RunQueue::with_low_fairness_window(3);
        queue.push_with_priority(1, Priority::Low);
        for pid in 10..20 {
            queue.push_with_priority(pid, Priority::High);
        }

        let popped: Vec<_> = (0..4).filter_map(|_| queue.pop()).collect();

        assert!(
            popped.contains(&1),
            "low priority pid did not progress: {popped:?}"
        );
    }

    #[test]
    fn steal_half_from_ten_takes_approximately_five() {
        let victim = RunQueue::new();
        for pid in 0..10 {
            victim.push(pid);
        }
        let stealer = victim.stealer();
        let thief = RunQueue::new();

        let stolen = thief.steal_half_from(&stealer);

        assert!((4..=6).contains(&stolen), "stole {stolen} items");
        assert!(!thief.is_empty());
        assert!(!victim.is_empty());
    }

    #[test]
    fn steal_from_empty_queue_returns_nothing() {
        let victim = RunQueue::new();
        let thief = RunQueue::new();

        assert_eq!(thief.steal_half_from(&victim.stealer()), 0);
        assert!(thief.is_empty());
    }

    #[test]
    fn steal_from_single_item_queue_returns_nothing() {
        let victim = RunQueue::new();
        victim.push(7);
        let thief = RunQueue::new();

        assert_eq!(thief.steal_half_from(&victim.stealer()), 0);
        assert_eq!(victim.len(), 1);
        assert!(thief.is_empty());
    }

    /// B-185 R1's red: the owner is running a slice, so its last queued
    /// process is stolen by the idle thief instead of waiting for the slice.
    #[test]
    fn steal_from_single_item_queue_of_an_owner_in_a_slice_takes_it() {
        let victim = RunQueue::new();
        victim.push(7);
        let in_slice = victim.enter_slice();
        let thief = RunQueue::new();

        assert_eq!(thief.steal_half_from(&victim.stealer()), 1);
        assert!(victim.is_empty());
        assert_eq!(thief.pop(), Some(7));

        // The slice returned: a victim of one is left alone again.
        drop(in_slice);
        victim.push(8);
        assert_eq!(thief.steal_half_from(&victim.stealer()), 0);
        assert_eq!(victim.len(), 1);
    }

    #[test]
    fn push_and_steal_from_different_threads_do_not_race() {
        let owner = RunQueue::new();
        for pid in 0..100 {
            owner.push(pid);
        }
        let stealer = owner.stealer();

        let thief_thread = std::thread::spawn(move || {
            let thief = RunQueue::new();
            let _stolen = thief.steal_half_from(&stealer);
            let mut items = Vec::new();
            while let Some(pid) = thief.pop() {
                items.push(pid);
            }
            items
        });

        let mut owner_items = Vec::new();
        while let Some(pid) = owner.pop() {
            owner_items.push(pid);
        }

        let thief_items = match thief_thread.join() {
            Ok(items) => items,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        let all: HashSet<_> = owner_items
            .iter()
            .chain(thief_items.iter())
            .copied()
            .collect();

        assert_eq!(all.len(), owner_items.len() + thief_items.len());
        assert!(all.len() <= 100);
    }
}
