//! The idle park of a normal scheduler worker, and its test instruments.
//!
//! EMBEDDER-COMPOSITION-SPEC §3.8 (B-181). This file holds the park itself
//! and the seams the tickless-idle tests read: per-worker park counts, the
//! timeout each worker last parked with, and (test builds only) hooks at the
//! idle gap and at the moment a worker starts to wait.

#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
#[cfg(feature = "telemetry")]
use std::time::Instant;

use crate::scheduler::{SharedState, lock_or_recover};

/// Idle-park timeout for a normal scheduler worker.
///
/// A worker with no runnable process parks on `wake_condvar` for at most this
/// long before looping again to re-check the two event sources still polled
/// rather than notified: timer-wheel expiry and IO-ring completions
/// (spec §3.8). It is the VM's own idle tick. Exposed as a named constant so
/// the signed §3.8 wake-rate bound reads its floor from source rather than a
/// duplicated literal; changing it re-signs the bound (Q-F ruling).
pub const IDLE_PARK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(5);

/// Per-worker idle wake rate implied by [`IDLE_PARK_TIMEOUT`], in wakes/sec.
///
/// The per-worker half of the signed §3.8 formula. The per-process bound is
/// this value times the worker count the inventory attributes to the process
/// — not a config-claimed count — so the signed number and its enforcement
/// cannot drift (Waffles the Terrible signing note; Vesper Lynd sharpening).
/// Consumed only by the signed idle-wake assertion, hence test-scoped.
#[cfg(any(test, feature = "test-support"))]
pub const IDLE_WAKES_PER_SEC_PER_WORKER: u64 = 1000u64 / IDLE_PARK_TIMEOUT.as_millis() as u64;

/// The timeout a worker most recently parked with, as the park seam saw it.
#[cfg(any(test, feature = "test-support"))]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ObservedParkTimeout {
    /// The worker has not parked yet.
    NotParked,
    /// The worker parked with no timeout: it waits for a wake edge only.
    Unbounded,
    /// The worker parked until a deadline this far ahead.
    Bounded(std::time::Duration),
}

#[cfg(any(test, feature = "test-support"))]
impl ObservedParkTimeout {
    const NOT_PARKED: u64 = 0;
    const UNBOUNDED: u64 = u64::MAX;

    fn encode(timeout: Option<std::time::Duration>) -> u64 {
        match timeout {
            None => Self::UNBOUNDED,
            Some(duration) => u64::try_from(duration.as_nanos())
                .unwrap_or(u64::MAX - 1)
                .saturating_add(1)
                .min(u64::MAX - 1),
        }
    }

    fn decode(encoded: u64) -> Self {
        match encoded {
            Self::NOT_PARKED => Self::NotParked,
            Self::UNBOUNDED => Self::Unbounded,
            nanos => Self::Bounded(std::time::Duration::from_nanos(nanos - 1)),
        }
    }

    /// Whole milliseconds of a bounded park, `None` for an unbounded park or
    /// none yet.
    #[must_use]
    pub fn bounded_millis(self) -> Option<u64> {
        match self {
            Self::Bounded(duration) => Some(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
            Self::NotParked | Self::Unbounded => None,
        }
    }
}

/// How many times one worker parked, and how each park ended.
#[cfg(any(test, feature = "test-support"))]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkerParkCounts {
    /// Entries into the park.
    pub entered: u64,
    /// Parks that ended because their deadline was reached.
    pub timed_out: u64,
    /// Parks that ended for any other reason: a wake edge, shutdown, or (at
    /// the base, before the wake sequence) any notify.
    pub woken: u64,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct WorkerParkSeam {
    entered: AtomicU64,
    timed_out: AtomicU64,
    woken: AtomicU64,
    last_timeout: AtomicU64,
}

/// Per-worker park instruments (`test-support`), sized to the worker count.
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler) struct ParkSeams {
    workers: Box<[WorkerParkSeam]>,
    last_timeout: AtomicU64,
}

#[cfg(any(test, feature = "test-support"))]
impl ParkSeams {
    pub(in crate::scheduler) fn new(worker_count: usize) -> Self {
        Self {
            workers: (0..worker_count.max(1))
                .map(|_| WorkerParkSeam::default())
                .collect(),
            last_timeout: AtomicU64::new(ObservedParkTimeout::NOT_PARKED),
        }
    }

    fn worker(&self, index: usize) -> Option<&WorkerParkSeam> {
        self.workers.get(index)
    }

    fn record_wait(&self, index: usize, timeout: Option<std::time::Duration>) {
        let encoded = ObservedParkTimeout::encode(timeout);
        self.last_timeout.store(encoded, Ordering::Release);
        if let Some(worker) = self.worker(index) {
            worker.last_timeout.store(encoded, Ordering::Release);
        }
    }

    pub(in crate::scheduler) fn counts(&self, index: usize) -> WorkerParkCounts {
        self.worker(index)
            .map(|worker| WorkerParkCounts {
                entered: worker.entered.load(Ordering::Acquire),
                timed_out: worker.timed_out.load(Ordering::Acquire),
                woken: worker.woken.load(Ordering::Acquire),
            })
            .unwrap_or_default()
    }

    pub(in crate::scheduler) fn observed(&self, index: usize) -> ObservedParkTimeout {
        self.worker(index).map_or(ObservedParkTimeout::NotParked, |worker| {
            ObservedParkTimeout::decode(worker.last_timeout.load(Ordering::Acquire))
        })
    }

    pub(in crate::scheduler) fn observed_any(&self) -> ObservedParkTimeout {
        ObservedParkTimeout::decode(self.last_timeout.load(Ordering::Acquire))
    }
}

/// A hook called with the worker index, at a point in the park sequence.
#[cfg(test)]
pub(in crate::scheduler) type WorkerHook = Box<dyn Fn(&SharedState, usize) + Send + Sync>;

/// A hook called just before a worker waits, with the `wait_set` lock HELD:
/// it must not take `wait_set` (or anything a wake source holds while taking
/// `wait_set`). Given the worker index and its timeout.
#[cfg(test)]
pub(in crate::scheduler) type WaitingHook =
    Box<dyn Fn(usize, Option<std::time::Duration>) + Send + Sync>;

/// Test-only controls on the park (B-181's reds).
#[cfg(test)]
pub(in crate::scheduler) struct ParkHooks {
    /// Called in the idle gap: after a worker's checks found nothing to run
    /// and before it enters the park. No lock is held.
    pub(in crate::scheduler) idle_gap: std::sync::OnceLock<WorkerHook>,
    /// Called as a worker starts to wait (see [`WaitingHook`]).
    pub(in crate::scheduler) waiting: std::sync::OnceLock<WaitingHook>,
    /// Called as a worker leaves its park, with what was pending for it
    /// (see [`LeftPark`]). No lock is held.
    pub(in crate::scheduler) left_park: std::sync::OnceLock<LeftParkHook>,
}

/// What a worker left its park with: the loss detector of B-181 R2's reds
/// (test builds only). A park that ended on its timeout with no wake edge
/// while work for this worker was pending is a lost wake that only the
/// clock recovered; with no timeout it would have parked for ever.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(in crate::scheduler) struct LeftPark {
    /// The worker that left its park.
    pub(in crate::scheduler) worker: usize,
    /// The park ended on its timeout, and no wake edge ran for this worker
    /// since its idle window opened.
    pub(in crate::scheduler) by_clock_without_edge: bool,
    /// Pids in `wait_set.woken` for this worker.
    pub(in crate::scheduler) woken: Vec<u64>,
    /// Spawns waiting in this worker's injection queue.
    pub(in crate::scheduler) injected: usize,
    /// `(worker, queued)` for every other worker whose run queue holds work
    /// this worker could steal.
    pub(in crate::scheduler) stealable: Vec<(usize, usize)>,
}

#[cfg(test)]
impl LeftPark {
    /// The lost wake this report shows, named, or `None`.
    pub(in crate::scheduler) fn lost_wake(&self) -> Option<String> {
        if !self.by_clock_without_edge {
            return None;
        }
        let mut pending = Vec::new();
        if !self.woken.is_empty() {
            pending.push(format!("pids {:?} in woken", self.woken));
        }
        if self.injected > 0 {
            pending.push(format!("{} spawns in its injection queue", self.injected));
        }
        for (owner, queued) in &self.stealable {
            pending.push(format!("{queued} stealable on worker {owner}'s run queue"));
        }
        if pending.is_empty() {
            return None;
        }
        Some(format!(
            "worker {} left its park on the clock with no wake edge while {} was pending: \
             the wake was lost",
            self.worker,
            pending.join(", ")
        ))
    }
}

/// The hook type of [`ParkHooks::left_park`].
#[cfg(test)]
pub(in crate::scheduler) type LeftParkHook = Box<dyn Fn(&LeftPark) + Send + Sync>;

#[cfg(test)]
impl Default for ParkHooks {
    fn default() -> Self {
        Self {
            idle_gap: std::sync::OnceLock::new(),
            waiting: std::sync::OnceLock::new(),
            left_park: std::sync::OnceLock::new(),
        }
    }
}

/// Run the idle-gap hook, if installed (test builds only).
#[cfg(test)]
pub(super) fn idle_gap(shared: &SharedState, my_index: usize) {
    if let Some(hook) = shared.park_hooks.idle_gap.get() {
        hook(shared, my_index);
    }
}

/// Park until a notify or the 5 ms timeout; returns whether the wait timed
/// out (the base's park, with B-181's seams).
pub(super) fn park_thread(shared: &SharedState, my_index: usize) -> bool {
    #[cfg(any(test, feature = "test-support"))]
    shared.idle_parks.fetch_add(1, Ordering::Relaxed);
    #[cfg(any(test, feature = "test-support"))]
    if let Some(worker) = shared.park_seams.worker(my_index) {
        worker.entered.fetch_add(1, Ordering::Relaxed);
    }
    if shared.shutdown.load(Ordering::Acquire) {
        return false;
    }
    let guard = lock_or_recover(&shared.wait_set);
    let timeout = Some(IDLE_PARK_TIMEOUT);
    // Record the SAME value the wait consumes: the seam observes the duration
    // actually passed to the wait, never a constant beside it.
    #[cfg(any(test, feature = "test-support"))]
    shared.park_seams.record_wait(my_index, timeout);
    #[cfg(test)]
    if let Some(hook) = shared.park_hooks.waiting.get() {
        hook(my_index, timeout);
    }
    #[cfg(feature = "telemetry")]
    let idle_started = Instant::now();
    let timed_out = match timeout {
        Some(timeout) => match shared.wake_condvar.wait_timeout(guard, timeout) {
            Ok((_guard, result)) => result.timed_out(),
            Err(error) => error.into_inner().1.timed_out(),
        },
        None => match shared.wake_condvar.wait(guard) {
            Ok(_guard) => false,
            Err(error) => {
                let _recovered = error.into_inner();
                false
            }
        },
    };
    #[cfg(any(test, feature = "test-support"))]
    if let Some(worker) = shared.park_seams.worker(my_index) {
        if timed_out {
            worker.timed_out.fetch_add(1, Ordering::Relaxed);
        } else {
            worker.woken.fetch_add(1, Ordering::Relaxed);
        }
    }
    #[cfg(feature = "telemetry")]
    shared.record_scheduler_idle(idle_started.elapsed());
    timed_out
}

/// Report how worker `my_index` left its park to the `left_park` hook, if
/// installed (test builds only). At the base there is no wake sequence: a
/// park that timed out had no notify, so "by the clock without an edge" is
/// the timeout itself.
#[cfg(test)]
pub(super) fn report_left_park(
    shared: &SharedState,
    my_index: usize,
    timed_out: bool,
    inject: &crossbeam_queue::SegQueue<crate::scheduler::SpawnRequest>,
    stealers: &[crate::scheduler::PriorityStealers],
) {
    let Some(hook) = shared.park_hooks.left_park.get() else {
        return;
    };
    let wait_set = lock_or_recover(&shared.wait_set);
    let woken = wait_set
        .woken
        .iter()
        .filter(|(_, worker)| *worker == my_index)
        .map(|(pid, _)| *pid)
        .collect();
    drop(wait_set);
    let stealable = stealers
        .iter()
        .enumerate()
        .filter(|(owner, _)| *owner != my_index)
        .map(|(owner, stealer)| (owner, stealer.queued()))
        .filter(|(_, queued)| *queued > 0)
        .collect();
    hook(&LeftPark {
        worker: my_index,
        by_clock_without_edge: timed_out,
        woken,
        injected: inject.len(),
        stealable,
    });
}

#[cfg(not(test))]
pub(super) fn report_left_park(
    _shared: &SharedState,
    _my_index: usize,
    _timed_out: bool,
    _inject: &crossbeam_queue::SegQueue<crate::scheduler::SpawnRequest>,
    _stealers: &[crate::scheduler::PriorityStealers],
) {
}
