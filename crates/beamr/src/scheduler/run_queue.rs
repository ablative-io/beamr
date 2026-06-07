//! Per-thread priority run queue backed by lock-free work-stealing deques.
//!
//! The owning scheduler thread pushes and pops from the back (LIFO for cache
//! locality). Stealers pop from the front (FIFO for fairness). Process IDs are
//! queued rather than process bodies because `Process` is intentionally `!Send`.

use std::cell::Cell;

use crossbeam_deque::{Steal, Stealer, Worker};

use crate::process::Priority;

/// Default number of non-low priority pops allowed before a queued low-priority
/// process is given a turn.
pub const DEFAULT_LOW_PRIORITY_INTERVAL: usize = 8;

/// Stealer handles for each priority queue belonging to one scheduler thread.
#[derive(Clone)]
pub struct PriorityStealer {
    max: Stealer<u64>,
    high: Stealer<u64>,
    normal: Stealer<u64>,
    low: Stealer<u64>,
}

/// A per-thread run queue that stores process IDs by process priority.
pub struct RunQueue {
    max: Worker<u64>,
    high: Worker<u64>,
    normal: Worker<u64>,
    low: Worker<u64>,
    low_priority_interval: usize,
    non_low_pops: Cell<usize>,
}

impl RunQueue {
    /// Create a new empty run queue with the default low-priority progress interval.
    #[must_use]
    pub fn new() -> Self {
        Self::with_low_priority_interval(DEFAULT_LOW_PRIORITY_INTERVAL)
    }

    /// Create a new empty run queue with a configured low-priority progress interval.
    ///
    /// An interval of zero is treated as one so a non-empty low-priority queue
    /// progresses immediately when higher-priority work is continuously present.
    #[must_use]
    pub fn with_low_priority_interval(low_priority_interval: usize) -> Self {
        Self {
            max: Worker::new_lifo(),
            high: Worker::new_lifo(),
            normal: Worker::new_lifo(),
            low: Worker::new_lifo(),
            low_priority_interval: low_priority_interval.max(1),
            non_low_pops: Cell::new(0),
        }
    }

    /// Push a process ID onto the owner side of the queue for `priority`.
    pub fn push(&self, pid: u64, priority: Priority) {
        self.worker(priority).push(pid);
    }

    /// Pop a process ID from the owner side of the queue.
    ///
    /// Normally this picks Max → High → Normal → Low. As an anti-starvation
    /// exception, after the configured number of non-low pops, a waiting
    /// low-priority process is popped before higher-priority processes.
    #[must_use]
    pub fn pop(&self) -> Option<u64> {
        if self.non_low_pops.get() >= self.low_priority_interval
            && let Some(pid) = self.low.pop()
        {
            self.non_low_pops.set(0);
            return Some(pid);
        }

        for priority in [Priority::Max, Priority::High, Priority::Normal] {
            if let Some(pid) = self.worker(priority).pop() {
                self.non_low_pops
                    .set(self.non_low_pops.get().saturating_add(1));
                return Some(pid);
            }
        }

        self.low.pop().inspect(|_| self.non_low_pops.set(0))
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
    pub fn stealer(&self) -> PriorityStealer {
        PriorityStealer {
            max: self.max.stealer(),
            high: self.high.stealer(),
            normal: self.normal.stealer(),
            low: self.low.stealer(),
        }
    }

    /// Steal approximately half the items from `victim` into this queue.
    ///
    /// Queues with zero or one item are left alone so the owning thread keeps
    /// its last runnable process. Stealing tries higher priorities first but
    /// keeps the caller's victim-selection algorithm unchanged.
    pub fn steal_half_from(&self, victim: &PriorityStealer) -> usize {
        self.steal_half_priority(&victim.max, Priority::Max)
            .or_else(|| self.steal_half_priority(&victim.high, Priority::High))
            .or_else(|| self.steal_half_priority(&victim.normal, Priority::Normal))
            .or_else(|| self.steal_half_priority(&victim.low, Priority::Low))
            .unwrap_or(0)
    }

    fn steal_half_priority(&self, victim: &Stealer<u64>, priority: Priority) -> Option<usize> {
        let victim_len = victim.len();
        if victim_len <= 1 {
            return None;
        }

        let limit = victim_len / 2;
        if limit == 0 {
            return None;
        }

        let worker = self.worker(priority);
        let before = worker.len();
        match victim.steal_batch_with_limit_and_pop(worker, limit) {
            Steal::Success(pid) => {
                worker.push(pid);
                Some(worker.len().saturating_sub(before))
            }
            Steal::Empty | Steal::Retry => None,
        }
    }

    fn worker(&self, priority: Priority) -> &Worker<u64> {
        match priority {
            Priority::Low => &self.low,
            Priority::Normal => &self.normal,
            Priority::High => &self.high,
            Priority::Max => &self.max,
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

    use super::RunQueue;
    use crate::process::Priority;

    #[test]
    fn push_then_pop_returns_same_process() {
        let queue = RunQueue::new();
        queue.push(42, Priority::Normal);

        assert_eq!(queue.pop(), Some(42));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn owner_pop_is_lifo_within_priority() {
        let queue = RunQueue::new();
        queue.push(1, Priority::Normal);
        queue.push(2, Priority::Normal);
        queue.push(3, Priority::Normal);

        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(1));
    }

    #[test]
    fn max_priority_dequeues_before_normal() {
        let queue = RunQueue::new();
        queue.push(1, Priority::Normal);
        queue.push(2, Priority::Max);

        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(1));
    }

    #[test]
    fn low_priority_process_makes_bounded_progress() {
        let queue = RunQueue::with_low_priority_interval(2);
        queue.push(1, Priority::Low);
        queue.push(10, Priority::High);
        queue.push(11, Priority::High);
        queue.push(12, Priority::High);

        assert_eq!(queue.pop(), Some(12));
        assert_eq!(queue.pop(), Some(11));
        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(10));
    }

    #[test]
    fn steal_half_from_ten_takes_approximately_five() {
        let victim = RunQueue::new();
        for pid in 0..10 {
            victim.push(pid, Priority::Normal);
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
        victim.push(7, Priority::Normal);
        let thief = RunQueue::new();

        assert_eq!(thief.steal_half_from(&victim.stealer()), 0);
        assert_eq!(victim.len(), 1);
        assert!(thief.is_empty());
    }

    #[test]
    fn push_and_steal_from_different_threads_do_not_race() {
        let owner = RunQueue::new();
        for pid in 0..100 {
            owner.push(pid, Priority::Normal);
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
