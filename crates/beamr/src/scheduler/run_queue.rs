//! Per-thread priority run queues backed by lock-free work-stealing deques.
//!
//! The owning scheduler thread pushes and pops from the back (LIFO for cache
//! locality). Stealers pop from the front (FIFO for fairness). Process IDs are
//! queued rather than process bodies because `Process` is intentionally `!Send`.

use std::cell::Cell;

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
}

/// A per-thread priority run queue that stores process IDs.
pub struct RunQueue {
    max: Worker<u64>,
    high: Worker<u64>,
    normal: Worker<u64>,
    low: Worker<u64>,
    low_fairness_window: usize,
    non_low_pops_since_low: Cell<usize>,
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
            max: Worker::new_lifo(),
            high: Worker::new_lifo(),
            normal: Worker::new_lifo(),
            low: Worker::new_lifo(),
            low_fairness_window,
            non_low_pops_since_low: Cell::new(0),
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

    /// Create stealer handles for other scheduler threads.
    #[must_use]
    pub fn stealer(&self) -> PriorityStealers {
        PriorityStealers {
            max: self.max.stealer(),
            high: self.high.stealer(),
            normal: self.normal.stealer(),
            low: self.low.stealer(),
        }
    }

    /// Steal approximately half the items from `victim` into this queue.
    ///
    /// Queues with zero or one total items are left alone so the owning thread
    /// keeps its last runnable process. Priority lanes are stolen independently;
    /// victim selection remains the caller's responsibility.
    pub fn steal_half_from(&self, victim: &PriorityStealers) -> usize {
        let victim_len = victim.len();
        if victim_len <= 1 {
            return 0;
        }

        let mut remaining = victim_len / 2;
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

    fn stealer(&self, priority: Priority) -> &Stealer<u64> {
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
        queue.push(42);

        assert_eq!(queue.pop(), Some(42));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn owner_pop_is_lifo_within_priority() {
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
