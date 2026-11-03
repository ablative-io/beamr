//! The seams the tickless-idle tests read (B-181): per-worker park counts,
//! the timeout each worker last parked with, the causes of worker 0's
//! deadline wakes, the waits on a worker parking, and (test builds only)
//! hooks at the idle gap, at the moment a worker starts to wait, and as it
//! leaves its park. No production path reads any of it.

use super::*;

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
            Self::Bounded(duration) => {
                Some(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
            }
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
    /// Parks that ended for any other reason: a wake edge or shutdown.
    pub woken: u64,
}

/// Why worker 0 woke by the clock, and how often replay cursor edges ran:
/// the causes the §3.8 bound counts.
#[cfg(any(test, feature = "test-support"))]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct WakeCauses {
    /// Worker 0 reached a deadline and its tick fired a timer.
    pub deadline_fired: u64,
    /// Worker 0 reached a deadline whose timer had been cancelled.
    pub deadline_cancelled: u64,
    /// Replay cursor edges.
    pub replay_cursor: u64,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct WorkerParkSeam {
    entered: AtomicU64,
    timed_out: AtomicU64,
    woken: AtomicU64,
    last_timeout: AtomicU64,
    /// Set while the worker waits; changed only under the `wait_set` mutex.
    waiting: std::sync::atomic::AtomicBool,
    /// B-184's seam: the worker panics at its next wait (taken once).
    panic_at_park: std::sync::atomic::AtomicBool,
    /// B-184's seam: the worker panics as it starts its next slice.
    panic_at_slice: std::sync::atomic::AtomicBool,
    /// Set, under the `wait_set` mutex, as a B-184 seam panics this worker.
    seam_panicked: std::sync::atomic::AtomicBool,
    /// B-184's hold: after the latch records this worker dead, its thread
    /// waits here until the [`WorkerHold`] is released.
    hold_after_death: std::sync::Mutex<Option<crossbeam_channel::Receiver<()>>>,
    /// B-184 R5's hold: after this worker died at its park or in a slice,
    /// its thread waits here, before its recovery checks anything, until
    /// the [`WorkerHold`] is released (test builds).
    #[cfg(test)]
    hold_before_recovery: std::sync::Mutex<Option<crossbeam_channel::Receiver<()>>>,
}

/// B-184's hold on a worker (test-support): while this is held, the worker's
/// thread, once the latch has recorded it dead, never ends. Dropping it, or
/// [`WorkerHold::release`], lets the thread go on and end.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub struct WorkerHold {
    _release: crossbeam_channel::Sender<()>,
}

#[cfg(any(test, feature = "test-support"))]
impl WorkerHold {
    /// Let the held worker's thread go on and end.
    pub fn release(self) {}
}

/// Per-worker park instruments (`test-support`), sized to the worker count.
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler) struct ParkSeams {
    workers: Box<[WorkerParkSeam]>,
    last_timeout: AtomicU64,
    /// Worker 0's deadline wakes whose tick fired at least one timer.
    pub(in crate::scheduler) deadline_fired: AtomicU64,
    /// Worker 0's deadline wakes whose tick fired nothing: the earliest
    /// timer was cancelled while worker 0 waited for it (W12).
    pub(in crate::scheduler) deadline_cancelled: AtomicU64,
    /// Replay cursor edges run (W24).
    pub(in crate::scheduler) replay_edges: AtomicU64,
}

#[cfg(any(test, feature = "test-support"))]
impl ParkSeams {
    pub(in crate::scheduler) fn new(worker_count: usize) -> Self {
        Self {
            workers: (0..worker_count.max(1))
                .map(|_| WorkerParkSeam::default())
                .collect(),
            last_timeout: AtomicU64::new(ObservedParkTimeout::NOT_PARKED),
            deadline_fired: AtomicU64::new(0),
            deadline_cancelled: AtomicU64::new(0),
            replay_edges: AtomicU64::new(0),
        }
    }

    /// Wake causes to date (B-181 R3's measurement instrument).
    pub(in crate::scheduler) fn causes(&self) -> WakeCauses {
        WakeCauses {
            deadline_fired: self.deadline_fired.load(Ordering::Acquire),
            deadline_cancelled: self.deadline_cancelled.load(Ordering::Acquire),
            replay_cursor: self.replay_edges.load(Ordering::Acquire),
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
        self.worker(index)
            .map_or(ObservedParkTimeout::NotParked, |worker| {
                ObservedParkTimeout::decode(worker.last_timeout.load(Ordering::Acquire))
            })
    }

    pub(in crate::scheduler) fn observed_any(&self) -> ObservedParkTimeout {
        ObservedParkTimeout::decode(self.last_timeout.load(Ordering::Acquire))
    }

    /// Arm B-184's park seam for worker `index`: false when no such worker.
    pub(in crate::scheduler) fn arm_panic_at_park(&self, index: usize) -> bool {
        self.worker(index).is_some_and(|worker| {
            worker.panic_at_park.store(true, Ordering::Release);
            true
        })
    }

    /// Arm B-184's hold for worker `index`: `None` when no such worker.
    pub(in crate::scheduler) fn arm_hold_after_death(&self, index: usize) -> Option<WorkerHold> {
        let worker = self.worker(index)?;
        let (release, held) = crossbeam_channel::bounded(0);
        *lock_or_recover(&worker.hold_after_death) = Some(held);
        Some(WorkerHold { _release: release })
    }

    /// Arm B-184 R5's hold before recovery for worker `index`: `None` when
    /// no such worker.
    #[cfg(test)]
    pub(in crate::scheduler) fn arm_hold_before_recovery(
        &self,
        index: usize,
    ) -> Option<WorkerHold> {
        let worker = self.worker(index)?;
        let (release, held) = crossbeam_channel::bounded(0);
        *lock_or_recover(&worker.hold_before_recovery) = Some(held);
        Some(WorkerHold { _release: release })
    }

    /// Arm B-184's slice seam for worker `index`: false when no such worker.
    pub(in crate::scheduler) fn arm_panic_at_slice(&self, index: usize) -> bool {
        self.worker(index).is_some_and(|worker| {
            worker.panic_at_slice.store(true, Ordering::Release);
            true
        })
    }
}

/// The message B-184's slice seam panics with in worker `index`, as it
/// starts a slice of `pid`.
#[cfg(any(test, feature = "test-support"))]
#[must_use]
pub fn slice_seam_panic_message(index: usize, pid: u64) -> String {
    format!("B-184 slice seam: beamr-sched-{index} panicked starting a slice of pid {pid}")
}

/// B-184's slice seam, run where the worker has popped `pid` and is about
/// to run it: the process is still `Present`, and whatever else the worker
/// queued stays on its local run queue. Panics once, when armed.
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler::execution) fn panic_if_armed_at_slice(
    shared: &SharedState,
    my_index: usize,
    pid: u64,
) {
    if let Some(worker) = shared.park_seams.worker(my_index)
        && worker.panic_at_slice.swap(false, Ordering::AcqRel)
    {
        let held = lock_or_recover(&shared.wait_set);
        worker.seam_panicked.store(true, Ordering::Release);
        shared.idle_wake.parked_changed.notify_all();
        drop(held);
        panic!("{}", slice_seam_panic_message(my_index, pid));
    }
}

/// B-184's hold, run by worker `my_index`'s guard after the latch recorded
/// it dead: waits, when armed, until its [`WorkerHold`] is dropped. Ends on
/// that event, never a clock.
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler) fn hold_if_armed(shared: &SharedState, my_index: usize) {
    let held = shared
        .park_seams
        .worker(my_index)
        .and_then(|worker| lock_or_recover(&worker.hold_after_death).take());
    if let Some(held) = held {
        // Disconnection is the release: the hold sends nothing.
        let _released = held.recv();
    }
}

/// Wake every waiter on the park instruments (the latch's wake), under the
/// `wait_set` mutex those waiters check their conditions under.
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler) fn notify_park_waiters(shared: &SharedState) {
    let held = lock_or_recover(&shared.wait_set);
    shared.idle_wake.parked_changed.notify_all();
    drop(held);
}

/// Block until a B-184 seam has panicked worker `worker`: ends on the
/// seam's mark, made under `wait_set` (test-support; never a clock). False
/// at once when there is no such worker.
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler) fn await_seam_panicked(shared: &SharedState, worker: usize) -> bool {
    let Some(seam) = shared.park_seams.worker(worker) else {
        return false;
    };
    let mut guard = lock_or_recover(&shared.wait_set);
    while !seam.seam_panicked.load(Ordering::Acquire) {
        guard = match shared.idle_wake.parked_changed.wait(guard) {
            Ok(guard) => guard,
            Err(error) => error.into_inner(),
        };
    }
    drop(guard);
    true
}

#[cfg(not(any(test, feature = "test-support")))]
pub(in crate::scheduler::execution) fn panic_if_armed_at_slice(
    _shared: &SharedState,
    _my_index: usize,
    _pid: u64,
) {
}

/// The message B-184's seam panics with in worker `index`'s park.
#[cfg(any(test, feature = "test-support"))]
#[must_use]
pub fn park_seam_panic_message(index: usize) -> String {
    format!(
        "B-184 park seam: beamr-sched-{index} panicked in its park, where std's condvar \
         assert ended the workers on macOS"
    )
}

/// B-184's seam, run where the worker is about to wait, with the `wait_set`
/// lock held and the worker counted as parked: the place std's condvar
/// assert panicked. Panics once, when armed for this worker.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn panic_if_armed(shared: &SharedState, my_index: usize) {
    if let Some(worker) = shared.park_seams.worker(my_index)
        && worker.panic_at_park.swap(false, Ordering::AcqRel)
    {
        // The caller holds `wait_set`: the mark and the notify are made under it.
        worker.seam_panicked.store(true, Ordering::Release);
        shared.idle_wake.parked_changed.notify_all();
        panic!("{}", park_seam_panic_message(my_index));
    }
}

#[cfg(not(any(test, feature = "test-support")))]
pub(super) fn panic_if_armed(_shared: &SharedState, _my_index: usize) {}

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

/// Report how worker `my_index` left its park to the `left_park` hook, if
/// installed (test builds only). "By the clock without an edge" is a park
/// that reached its deadline while the wake sequence still read what the
/// worker read entering its window.
#[cfg(test)]
pub(in crate::scheduler::execution) fn report_left_park(
    shared: &SharedState,
    my_index: usize,
    exit: ParkExit,
    ticket: IdleTicket,
    inject: &crossbeam_queue::SegQueue<crate::scheduler::SpawnRequest>,
    stealers: &[crate::scheduler::PriorityStealers],
) {
    let Some(hook) = shared.park_hooks.left_park.get() else {
        return;
    };
    let wait_set = lock_or_recover(&shared.wait_set);
    let by_clock_without_edge = exit == ParkExit::Deadline
        && shared.idle_wake.sequence.load(Ordering::Acquire) == ticket.sequence;
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
        by_clock_without_edge,
        woken,
        injected: inject.len(),
        stealable,
    });
}

/// Run the idle-gap hook, if installed (test builds only).
#[cfg(test)]
pub(in crate::scheduler::execution) fn idle_gap(shared: &SharedState, my_index: usize) {
    if let Some(hook) = shared.park_hooks.idle_gap.get() {
        hook(shared, my_index);
    }
}

#[cfg(any(test, feature = "test-support"))]
impl IdleWake {
    pub(in crate::scheduler) fn idle_workers(&self) -> usize {
        self.idle_workers.load(Ordering::SeqCst)
    }

    pub(in crate::scheduler) fn parked_workers(&self) -> usize {
        self.parked_workers.load(Ordering::Acquire)
    }

    pub(in crate::scheduler) fn edges(&self) -> u64 {
        self.edges.load(Ordering::Acquire)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn count_entry(shared: &SharedState, my_index: usize) {
    shared.idle_parks.fetch_add(1, Ordering::Relaxed);
    if let Some(worker) = shared.park_seams.worker(my_index) {
        worker.entered.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(not(any(test, feature = "test-support")))]
pub(super) fn count_entry(_shared: &SharedState, _my_index: usize) {}

/// The park seams' view of a wait about to start: the timeout actually
/// passed to the wait (never a constant beside it), and the test hook.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn note_wait(shared: &SharedState, my_index: usize, deadline: Option<Instant>) {
    let timeout = deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
    shared.park_seams.record_wait(my_index, timeout);
    #[cfg(test)]
    if let Some(hook) = shared.park_hooks.waiting.get() {
        hook(my_index, timeout);
    }
    if let Some(worker) = shared.park_seams.worker(my_index) {
        worker.waiting.store(true, Ordering::Release);
    }
    shared
        .idle_wake
        .parked_workers
        .fetch_add(1, Ordering::AcqRel);
    shared.idle_wake.parked_changed.notify_all();
}

/// The park seams' view of a wait that ended (under the `wait_set` mutex).
#[cfg(any(test, feature = "test-support"))]
pub(super) fn note_wait_end(shared: &SharedState, my_index: usize) {
    if let Some(worker) = shared.park_seams.worker(my_index) {
        worker.waiting.store(false, Ordering::Release);
    }
    shared
        .idle_wake
        .parked_workers
        .fetch_sub(1, Ordering::AcqRel);
}

#[cfg(not(any(test, feature = "test-support")))]
pub(super) fn note_wait_end(_shared: &SharedState, _my_index: usize) {}

/// Block until worker `worker` is waiting in a park it entered after
/// `since` was read, and answer the timeout that park waits with. Ends on a
/// worker starting to wait (test-support; never a clock).
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler) fn await_worker_parked(
    shared: &SharedState,
    worker: usize,
    since: WorkerParkCounts,
) -> ObservedParkTimeout {
    let Some(seam) = shared.park_seams.worker(worker) else {
        return ObservedParkTimeout::NotParked;
    };
    let mut guard = lock_or_recover(&shared.wait_set);
    loop {
        if (seam.waiting.load(Ordering::Acquire)
            && seam.entered.load(Ordering::Acquire) > since.entered)
            || shared.failure.is_latched()
        {
            return ObservedParkTimeout::decode(seam.last_timeout.load(Ordering::Acquire));
        }
        guard = match shared.idle_wake.parked_changed.wait(guard) {
            Ok(guard) => guard,
            Err(error) => error.into_inner(),
        };
    }
}

/// Block until every worker is waiting at once.
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler) fn await_all_parked(shared: &SharedState) {
    let mut guard = lock_or_recover(&shared.wait_set);
    while shared.idle_wake.parked_workers.load(Ordering::Acquire) < shared.thread_count
        && !shared.failure.is_latched()
    {
        guard = match shared.idle_wake.parked_changed.wait(guard) {
            Ok(guard) => guard,
            Err(error) => error.into_inner(),
        };
    }
}

#[cfg(not(any(test, feature = "test-support")))]
pub(super) fn note_wait(_shared: &SharedState, _my_index: usize, _deadline: Option<Instant>) {}

/// Count why worker 0 woke by the clock: its tick after a deadline wake
/// either fired a timer, or found the earliest one cancelled (W12).
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler::execution) fn account_deadline_wake(
    shared: &SharedState,
    woke_on_deadline: bool,
    fired: usize,
) {
    if !woke_on_deadline {
        return;
    }
    let counter = if fired > 0 {
        &shared.park_seams.deadline_fired
    } else {
        &shared.park_seams.deadline_cancelled
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

#[cfg(not(any(test, feature = "test-support")))]
pub(in crate::scheduler::execution) fn account_deadline_wake(
    _shared: &SharedState,
    _woke_on_deadline: bool,
    _fired: usize,
) {
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn record_outcome(shared: &SharedState, my_index: usize, outcome: ParkOutcome) {
    if let Some(worker) = shared.park_seams.worker(my_index) {
        match outcome {
            ParkOutcome::TimedOut => worker.timed_out.fetch_add(1, Ordering::Relaxed),
            ParkOutcome::Woken => worker.woken.fetch_add(1, Ordering::Relaxed),
        };
    }
}

#[cfg(not(any(test, feature = "test-support")))]
pub(super) fn record_outcome(_shared: &SharedState, _my_index: usize, _outcome: ParkOutcome) {}

/// The message B-184 R5's poisoning thread panics with.
#[cfg(any(test, feature = "test-support"))]
const LINK_SET_POISON: &str =
    "B-184 R5 seam: link_set poisoned, so no recovery can prove the scheduler's state";

/// B-184 R5's poison: a thread panics holding `link_set`, which poisons it;
/// joined here, so it ends on the thread's exit, never a clock. True when
/// `link_set` is poisoned afterwards.
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler) fn poison_link_set(shared: &SharedState) -> bool {
    std::thread::scope(|scope| {
        let poisoner = std::thread::Builder::new()
            .name("beamr-b184-poison".to_string())
            .spawn_scoped(scope, || {
                let _held = lock_or_recover(&shared.link_set);
                panic!("{LINK_SET_POISON}");
            });
        if let Ok(poisoner) = poisoner {
            let _panicked = poisoner.join();
        }
    });
    shared.link_set.is_poisoned()
}

/// B-184 R5's hold, run by worker `my_index`'s guard after it died and
/// before its recovery checks anything: waits, when armed, until its
/// [`WorkerHold`] is dropped. Ends on that event, never a clock.
#[cfg(test)]
pub(in crate::scheduler) fn hold_before_recovery_if_armed(shared: &SharedState, my_index: usize) {
    let held = shared
        .park_seams
        .worker(my_index)
        .and_then(|worker| lock_or_recover(&worker.hold_before_recovery).take());
    if let Some(held) = held {
        // Disconnection is the release: the hold sends nothing.
        let _released = held.recv();
    }
}
