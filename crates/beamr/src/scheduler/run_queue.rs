//! Per-thread run queue backed by a lock-free work-stealing deque.
//!
//! The owning scheduler thread pushes and pops from the back (LIFO for cache
//! locality). Stealers pop from the front (FIFO for fairness). Process IDs are
//! queued rather than process bodies because `Process` is intentionally `!Send`.

use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_deque::{Steal, Stealer, Worker};

const FAIR_PRIORITY_POP_LIMIT: usize = 8;

/// Scheduler priority for a process run-queue entry.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ProcessPriority {
    /// Normal-priority process.
    #[default]
    Normal,
    /// High-priority process, ahead of normal work.
    High,
    /// Max-priority process, ahead of high and normal work.
    Max,
}

/// Stealer handles for all priority lanes of a run queue.
#[derive(Clone)]
pub struct RunQueueStealers {
    max: Stealer<u64>,
    high: Stealer<u64>,
    normal: Stealer<u64>,
}

/// A per-thread run queue that stores process IDs.
pub struct RunQueue {
    max: Worker<u64>,
    high: Worker<u64>,
    normal: Worker<u64>,
    priority_pops_since_normal: AtomicUsize,
}

impl RunQueue {
    /// Create a new empty run queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max: Worker::new_lifo(),
            high: Worker::new_lifo(),
            normal: Worker::new_lifo(),
            priority_pops_since_normal: AtomicUsize::new(0),
        }
    }

    /// Push a normal-priority process ID onto the owner side of the queue.
    pub fn push(&self, pid: u64) {
        self.push_with_priority(pid, ProcessPriority::Normal);
    }

    /// Push a process ID onto the owner side of the queue at `priority`.
    pub fn push_with_priority(&self, pid: u64, priority: ProcessPriority) {
        match priority {
            ProcessPriority::Normal => self.normal.push(pid),
            ProcessPriority::High => self.high.push(pid),
            ProcessPriority::Max => self.max.push(pid),
        }
    }

    /// Pop a process ID from the owner side of the queue.
    #[must_use]
    pub fn pop(&self) -> Option<u64> {
        if self.priority_pops_since_normal.load(Ordering::Relaxed) >= FAIR_PRIORITY_POP_LIMIT
            && let Some(pid) = self.normal.pop()
        {
            self.priority_pops_since_normal.store(0, Ordering::Relaxed);
            return Some(pid);
        }

        if let Some(pid) = self.max.pop() {
            self.priority_pops_since_normal
                .fetch_add(1, Ordering::Relaxed);
            return Some(pid);
        }
        if let Some(pid) = self.high.pop() {
            self.priority_pops_since_normal
                .fetch_add(1, Ordering::Relaxed);
            return Some(pid);
        }
        if let Some(pid) = self.normal.pop() {
            self.priority_pops_since_normal.store(0, Ordering::Relaxed);
            return Some(pid);
        }
        None
    }

    /// Approximate number of queued process IDs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.max.len() + self.high.len() + self.normal.len()
    }

    /// Whether this queue is currently empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.max.is_empty() && self.high.is_empty() && self.normal.is_empty()
    }

    /// Create stealer handles for other scheduler threads.
    #[must_use]
    pub fn stealer(&self) -> RunQueueStealers {
        RunQueueStealers {
            max: self.max.stealer(),
            high: self.high.stealer(),
            normal: self.normal.stealer(),
        }
    }

    /// Steal approximately half the items from `victim` into this queue.
    ///
    /// Queues with zero or one item are left alone so the owning thread keeps
    /// its last runnable process. Higher priority lanes are tried first.
    pub fn steal_half_from(&self, victim: &RunQueueStealers) -> usize {
        self.steal_lane(&victim.max, &self.max)
            .or_else(|| self.steal_lane(&victim.high, &self.high))
            .or_else(|| self.steal_lane(&victim.normal, &self.normal))
            .unwrap_or(0)
    }

    fn steal_lane(&self, victim: &Stealer<u64>, target: &Worker<u64>) -> Option<usize> {
        let victim_len = victim.len();
        if victim_len <= 1 {
            return None;
        }

        let limit = victim_len / 2;
        if limit == 0 {
            return None;
        }

        let before = target.len();
        match victim.steal_batch_with_limit_and_pop(target, limit) {
            Steal::Success(pid) => {
                target.push(pid);
                Some(target.len().saturating_sub(before))
            }
            Steal::Empty | Steal::Retry => None,
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

    use super::{ProcessPriority, RunQueue};

    #[test]
    fn push_then_pop_returns_same_process() {
        let queue = RunQueue::new();
        queue.push(42);

        assert_eq!(queue.pop(), Some(42));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn owner_pop_is_lifo() {
        let queue = RunQueue::new();
        queue.push(1);
        queue.push(2);
        queue.push(3);

        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(1));
    }

    #[test]
    fn priority_order_is_max_then_high_then_normal() {
        let queue = RunQueue::new();
        queue.push_with_priority(1, ProcessPriority::Normal);
        queue.push_with_priority(2, ProcessPriority::High);
        queue.push_with_priority(3, ProcessPriority::Max);

        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(1));
    }

    #[test]
    fn normal_priority_runs_within_bounded_priority_window() {
        let queue = RunQueue::new();
        queue.push_with_priority(1, ProcessPriority::Normal);
        for pid in 10..30 {
            queue.push_with_priority(pid, ProcessPriority::High);
        }

        let mut seen_normal = false;
        for _round in 0..10 {
            seen_normal |= queue.pop() == Some(1);
            if seen_normal {
                break;
            }
        }
        assert!(
            seen_normal,
            "normal priority process should run within 10 pops"
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
