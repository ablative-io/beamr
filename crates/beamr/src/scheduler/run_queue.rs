//! Per-thread priority run queue backed by lock-free work-stealing deques.
//!
//! The owning scheduler thread pushes and pops from the back of each priority
//! queue (LIFO for cache locality). Stealers pop from the front (FIFO for
//! fairness). Process IDs are queued rather than process bodies because
//! `Process` is intentionally `!Send`.

use std::cell::Cell;

use crossbeam_deque::{Steal, Stealer, Worker};

use crate::process::Priority;

/// Default owner-pop scheduling window within which a queued low-priority item
/// is guaranteed to be preferred once when non-low work is also available.
pub const DEFAULT_LOW_PRIORITY_INTERVAL: usize = 8;

/// Composite stealer handle for a priority-aware run queue.
#[derive(Clone)]
pub struct RunQueueStealer {
    max: Stealer<u64>,
    high: Stealer<u64>,
    normal: Stealer<u64>,
    low: Stealer<u64>,
}

impl RunQueueStealer {
    fn stealer_for(&self, priority: Priority) -> &Stealer<u64> {
        match priority {
            Priority::Max => &self.max,
            Priority::High => &self.high,
            Priority::Normal => &self.normal,
            Priority::Low => &self.low,
        }
    }
}

/// A per-thread run queue that stores process IDs by scheduling priority.
pub struct RunQueue {
    max: Worker<u64>,
    high: Worker<u64>,
    normal: Worker<u64>,
    low: Worker<u64>,
    low_priority_interval: usize,
    non_low_pops_since_low: Cell<usize>,
}

impl RunQueue {
    /// Create a new empty run queue.
    #[must_use]
    pub fn new() -> Self {
        Self::with_low_priority_interval(DEFAULT_LOW_PRIORITY_INTERVAL)
    }

    /// Create a new empty run queue with a custom low-priority progress window.
    #[must_use]
    pub fn with_low_priority_interval(low_priority_interval: usize) -> Self {
        Self {
            max: Worker::new_lifo(),
            high: Worker::new_lifo(),
            normal: Worker::new_lifo(),
            low: Worker::new_lifo(),
            low_priority_interval: low_priority_interval.max(1),
            non_low_pops_since_low: Cell::new(0),
        }
    }

    /// Push a process ID at normal priority onto the owner side of the queue.
    pub fn push(&self, pid: u64) {
        self.push_with_priority(pid, Priority::Normal);
    }

    /// Push a process ID at `priority` onto the owner side of the queue.
    pub fn push_with_priority(&self, pid: u64, priority: Priority) {
        self.worker_for(priority).push(pid);
    }

    /// Pop a process ID from the owner side of the highest-priority available queue.
    #[must_use]
    pub fn pop(&self) -> Option<u64> {
        if self.non_low_pops_since_low.get() + 1 >= self.low_priority_interval
            && let Some(pid) = self.pop_low()
        {
            return Some(pid);
        }

        for priority in [
            Priority::Max,
            Priority::High,
            Priority::Normal,
            Priority::Low,
        ] {
            if let Some(pid) = self.worker_for(priority).pop() {
                self.record_pop(priority);
                return Some(pid);
            }
        }
        None
    }

    /// Approximate number of queued process IDs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.max.len() + self.high.len() + self.normal.len() + self.low.len()
    }

    /// Whether this queue is currently empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.max.is_empty() && self.high.is_empty() && self.normal.is_empty() && self.low.is_empty()
    }

    /// Create a stealer handle for other scheduler threads.
    #[must_use]
    pub fn stealer(&self) -> RunQueueStealer {
        RunQueueStealer {
            max: self.max.stealer(),
            high: self.high.stealer(),
            normal: self.normal.stealer(),
            low: self.low.stealer(),
        }
    }

    /// Steal approximately half the items from `victim` into this queue.
    ///
    /// Queues with zero or one item are left alone so the owning thread keeps
    /// its last runnable process. Priority queues are stolen independently in
    /// highest-to-lowest order without changing victim scheduler selection.
    pub fn steal_half_from(&self, victim: &RunQueueStealer) -> usize {
        let mut stolen = 0;
        for priority in [
            Priority::Max,
            Priority::High,
            Priority::Normal,
            Priority::Low,
        ] {
            stolen += self.steal_priority_half_from(victim.stealer_for(priority), priority);
        }
        stolen
    }

    fn worker_for(&self, priority: Priority) -> &Worker<u64> {
        match priority {
            Priority::Max => &self.max,
            Priority::High => &self.high,
            Priority::Normal => &self.normal,
            Priority::Low => &self.low,
        }
    }

    fn pop_low(&self) -> Option<u64> {
        let pid = self.low.pop()?;
        self.record_pop(Priority::Low);
        Some(pid)
    }

    fn record_pop(&self, priority: Priority) {
        if priority == Priority::Low {
            self.non_low_pops_since_low.set(0);
        } else {
            self.non_low_pops_since_low
                .set(self.non_low_pops_since_low.get().saturating_add(1));
        }
    }

    fn steal_priority_half_from(&self, victim: &Stealer<u64>, priority: Priority) -> usize {
        let victim_len = victim.len();
        if victim_len <= 1 {
            return 0;
        }

        let limit = victim_len / 2;
        if limit == 0 {
            return 0;
        }

        let worker = self.worker_for(priority);
        let before = worker.len();
        match victim.steal_batch_with_limit_and_pop(worker, limit) {
            Steal::Success(pid) => {
                worker.push(pid);
                worker.len().saturating_sub(before)
            }
            Steal::Empty | Steal::Retry => 0,
        }
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

    use super::{DEFAULT_LOW_PRIORITY_INTERVAL, RunQueue};
    use crate::process::Priority;

    #[test]
    fn push_then_pop_returns_same_process() {
        let queue = RunQueue::new();
        queue.push(42);

        assert_eq!(queue.pop(), Some(42));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn owner_pop_is_lifo_within_same_priority() {
        let queue = RunQueue::new();
        queue.push(1);
        queue.push(2);
        queue.push(3);

        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(1));
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
    fn low_priority_makes_progress_within_default_interval_amid_high_priority_work() {
        let queue = RunQueue::new();
        queue.push_with_priority(1, Priority::Low);
        for pid in 2..=20 {
            queue.push_with_priority(pid, Priority::High);
        }

        let mut popped = Vec::new();
        for _ in 0..DEFAULT_LOW_PRIORITY_INTERVAL {
            if let Some(pid) = queue.pop() {
                popped.push(pid);
            }
        }

        assert!(
            popped.contains(&1),
            "low priority pid did not make progress within {DEFAULT_LOW_PRIORITY_INTERVAL} pops: {popped:?}"
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
