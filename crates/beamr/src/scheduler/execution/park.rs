//! The idle park of a normal scheduler worker, and its test instruments.
//!
//! EMBEDDER-COMPOSITION-SPEC §3.8 (B-181). An idle worker parks with no
//! timeout; worker 0 parks only until the earliest armed timer, the one
//! source it must reach by the clock. Every other source runs edge E (take
//! the `wait_set` lock, advance the wake sequence, `notify_all`), and the
//! park waits only while, under that lock, the sequence is the one the
//! worker read before its checks — so no wake is lost and none is polled.
//!
//! This file also holds the seams the tickless-idle tests read: per-worker
//! park counts, the timeout each worker last parked with, the causes of
//! worker 0's deadline wakes, and (test builds only) hooks at the idle gap
//! and at the moment a worker starts to wait.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering, fence};
use std::time::Instant;

use crate::scheduler::{RunQueue, SharedState, WaitSet, lock_or_recover};

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

/// Report how worker `my_index` left its park to the `left_park` hook, if
/// installed (test builds only). "By the clock without an edge" is a park
/// that reached its deadline while the wake sequence still read what the
/// worker read entering its window.
#[cfg(test)]
pub(super) fn report_left_park(
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
pub(super) fn idle_gap(shared: &SharedState, my_index: usize) {
    if let Some(hook) = shared.park_hooks.idle_gap.get() {
        hook(shared, my_index);
    }
}

/// The wake sequence and the idle count (B-181 R2, §3.8.2 C1).
///
/// `sequence` is advanced by edge E, always under the `wait_set` mutex
/// (Release), and read by a worker before its checks (Acquire) and again
/// under the mutex before it waits. `idle_workers` counts workers inside
/// their idle window: raised before the sequence read, lowered on every way
/// out of the window (work found, or the park left).
pub(in crate::scheduler) struct IdleWake {
    sequence: AtomicU64,
    idle_workers: AtomicUsize,
    /// Workers waiting on `wake_condvar` right now, changed only under the
    /// `wait_set` mutex (instrument for C1's idle-count red).
    #[cfg(any(test, feature = "test-support"))]
    parked_workers: AtomicUsize,
    /// Edges E run to date (measurement instrument).
    #[cfg(any(test, feature = "test-support"))]
    edges: AtomicU64,
    /// Notified, under the `wait_set` mutex, each time a worker starts to
    /// wait: what a test blocks on to learn a worker has parked.
    #[cfg(any(test, feature = "test-support"))]
    parked_changed: std::sync::Condvar,
}

impl Default for IdleWake {
    fn default() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            idle_workers: AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-support"))]
            parked_workers: AtomicUsize::new(0),
            #[cfg(any(test, feature = "test-support"))]
            edges: AtomicU64::new(0),
            #[cfg(any(test, feature = "test-support"))]
            parked_changed: std::sync::Condvar::new(),
        }
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

impl SharedState {
    /// Edge E's advance, made under the `wait_set` mutex: the `held` borrow
    /// is the proof the caller holds it. A source that pushes to
    /// `wait_set.woken` advances in that same critical section.
    pub(in crate::scheduler) fn advance_wake_sequence(&self, _held: &WaitSet) {
        self.idle_wake.sequence.fetch_add(1, Ordering::Release);
        #[cfg(any(test, feature = "test-support"))]
        self.idle_wake.edges.fetch_add(1, Ordering::Relaxed);
    }

    /// Edge E for a source already holding the `wait_set` mutex: advance the
    /// wake sequence, then `notify_all`. Every wake stays `notify_all`
    /// (§3.8.1's invariant: only worker 0 ticks timers and drains file IO).
    pub(in crate::scheduler) fn wake_edge_locked(&self, held: &WaitSet) {
        self.advance_wake_sequence(held);
        self.wake_condvar.notify_all();
    }

    /// Edge E for a source holding no lock: take `wait_set`, advance, notify.
    pub(in crate::scheduler) fn wake_edge(&self) {
        let held = lock_or_recover(&self.wait_set);
        self.wake_edge_locked(&held);
    }
}

/// What a worker read when it entered its idle window.
#[derive(Copy, Clone, Debug)]
pub(super) struct IdleTicket {
    sequence: u64,
}

/// Enter the idle window, before the worker's checks: raise the idle count
/// (SeqCst RMW), then a SeqCst fence, then read the wake sequence (Acquire).
/// The fence is the worker's half of C1's Dekker pattern: it keeps the
/// checks and the steal after the raise (on ARM64 an Acquire load could
/// otherwise move ahead of the RMW's store).
pub(super) fn enter_idle_window(shared: &SharedState) -> IdleTicket {
    shared.idle_wake.idle_workers.fetch_add(1, Ordering::SeqCst);
    fence(Ordering::SeqCst);
    IdleTicket {
        sequence: shared.idle_wake.sequence.load(Ordering::Acquire),
    }
}

/// Leave the idle window. Called on EVERY way out: when the worker found
/// work without parking, and when it leaves the park.
pub(super) fn leave_idle_window(shared: &SharedState) {
    shared.idle_wake.idle_workers.fetch_sub(1, Ordering::SeqCst);
}

/// W23's surplus edge (§3.8.2 C1), run by a worker right after it pushed to
/// its own run queue. The pusher's half of the Dekker pattern: fence, then
/// read the idle count; run E when another worker is idle and the queue
/// holds more than the process the pusher is about to run (a queue of one
/// is never stolen from, so a Requeue with nothing behind it is no
/// surplus). `pusher_in_window` says whether the pusher itself is counted.
pub(in crate::scheduler) fn surplus_edge(
    shared: &SharedState,
    queue: &RunQueue,
    pusher_in_window: bool,
) {
    fence(Ordering::SeqCst);
    let idle = shared.idle_wake.idle_workers.load(Ordering::SeqCst);
    let others_idle = idle.saturating_sub(usize::from(pusher_in_window));
    if others_idle > 0 && queue.len() >= 2 {
        shared.wake_edge();
    }
}

/// How a park ended.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ParkOutcome {
    /// The wake sequence moved, or shutdown was raised: back to the loop.
    Woken,
    /// The park's deadline was reached.
    TimedOut,
}

/// Park an idle worker until edge E moves the wake sequence past what the
/// worker read entering its window, or the park's deadline. Leaves the idle
/// window on every path.
pub(super) fn park_thread(shared: &SharedState, my_index: usize, ticket: IdleTicket) -> ParkExit {
    let exit = if shared.replay_mode {
        replay_park(shared, my_index, ticket)
    } else {
        live_park(shared, my_index, ticket)
    };
    leave_idle_window(shared);
    exit
}

/// How a live park ended, for worker 0's wake accounting.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) enum ParkExit {
    /// Back to the loop on an edge or shutdown.
    Woken,
    /// Worker 0 reached its published deadline.
    Deadline,
}

#[cfg(any(test, feature = "test-support"))]
fn count_entry(shared: &SharedState, my_index: usize) {
    shared.idle_parks.fetch_add(1, Ordering::Relaxed);
    if let Some(worker) = shared.park_seams.worker(my_index) {
        worker.entered.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(not(any(test, feature = "test-support")))]
fn count_entry(_shared: &SharedState, _my_index: usize) {}

/// The park seams' view of a wait about to start: the timeout actually
/// passed to the wait (never a constant beside it), and the test hook.
#[cfg(any(test, feature = "test-support"))]
fn note_wait(shared: &SharedState, my_index: usize, deadline: Option<Instant>) {
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
fn note_wait_end(shared: &SharedState, my_index: usize) {
    if let Some(worker) = shared.park_seams.worker(my_index) {
        worker.waiting.store(false, Ordering::Release);
    }
    shared
        .idle_wake
        .parked_workers
        .fetch_sub(1, Ordering::AcqRel);
}

#[cfg(not(any(test, feature = "test-support")))]
fn note_wait_end(_shared: &SharedState, _my_index: usize) {}

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
        if seam.waiting.load(Ordering::Acquire)
            && seam.entered.load(Ordering::Acquire) > since.entered
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
    while shared.idle_wake.parked_workers.load(Ordering::Acquire) < shared.thread_count {
        guard = match shared.idle_wake.parked_changed.wait(guard) {
            Ok(guard) => guard,
            Err(error) => error.into_inner(),
        };
    }
}

#[cfg(not(any(test, feature = "test-support")))]
fn note_wait(_shared: &SharedState, _my_index: usize, _deadline: Option<Instant>) {}

fn live_park(shared: &SharedState, my_index: usize, ticket: IdleTicket) -> ParkExit {
    count_entry(shared, my_index);
    let deadline = park_deadline(shared, my_index);
    let outcome = wait_for_edge(shared, my_index, ticket, deadline);
    clear_park(shared, my_index);
    record_outcome(shared, my_index, outcome);
    match outcome {
        ParkOutcome::Woken => ParkExit::Woken,
        ParkOutcome::TimedOut => ParkExit::Deadline,
    }
}

/// W24: the replay park. The log, never the clock, decides when a replay
/// worker leaves: worker 0 leaves unless the cursor holds another worker's
/// `Schedule` (its next tick consumes or refuses anything else exactly as
/// before); worker i > 0 leaves only when the cursor holds its own
/// `Schedule` for a pid in the process table. A wake that leaves the cursor
/// short of that re-reads the sequence, re-checks the cursor and parks
/// again, without running a loop iteration (every worker-0 iteration
/// consumes a log event).
fn replay_park(shared: &SharedState, my_index: usize, ticket: IdleTicket) -> ParkExit {
    let mut ticket = ticket;
    loop {
        if shared.shutdown.load(Ordering::Acquire) || replay_turn(shared, my_index) {
            return ParkExit::Woken;
        }
        count_entry(shared, my_index);
        let outcome = wait_for_edge(shared, my_index, ticket, None);
        record_outcome(shared, my_index, outcome);
        // The new sequence is read BEFORE the cursor check: an advance after
        // the check runs its edge after this read, and ends the next wait.
        ticket = IdleTicket {
            sequence: shared.idle_wake.sequence.load(Ordering::Acquire),
        };
    }
}

fn replay_turn(shared: &SharedState, my_index: usize) -> bool {
    let Some(driver) = shared.replay_driver.as_ref() else {
        return true;
    };
    let guard = lock_or_recover(driver);
    match guard.peek_schedule() {
        Some(event) if my_index == 0 => event.scheduler_index == 0,
        None if my_index == 0 => true,
        Some(event) => {
            event.scheduler_index == my_index && shared.process_table.get(event.pid).is_some()
        }
        None => false,
    }
}

/// Count why worker 0 woke by the clock: its tick after a deadline wake
/// either fired a timer, or found the earliest one cancelled (W12).
#[cfg(any(test, feature = "test-support"))]
pub(super) fn account_deadline_wake(shared: &SharedState, woke_on_deadline: bool, fired: usize) {
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
pub(super) fn account_deadline_wake(_shared: &SharedState, _woke_on_deadline: bool, _fired: usize) {
}

/// W24's edge: run after a replay step that advanced the cursor (worker 0's
/// timer tick, and every replay slice), with no driver lock held. The
/// advances inside a slice are made at the interpreter's and the natives'
/// own lock sites; one edge after the slice covers all of them.
pub(super) fn replay_cursor_edge(shared: &SharedState) {
    #[cfg(any(test, feature = "test-support"))]
    shared
        .park_seams
        .replay_edges
        .fetch_add(1, Ordering::Relaxed);
    shared.wake_edge();
}

#[cfg(any(test, feature = "test-support"))]
fn record_outcome(shared: &SharedState, my_index: usize, outcome: ParkOutcome) {
    if let Some(worker) = shared.park_seams.worker(my_index) {
        match outcome {
            ParkOutcome::TimedOut => worker.timed_out.fetch_add(1, Ordering::Relaxed),
            ParkOutcome::Woken => worker.woken.fetch_add(1, Ordering::Relaxed),
        };
    }
}

#[cfg(not(any(test, feature = "test-support")))]
fn record_outcome(_shared: &SharedState, _my_index: usize, _outcome: ParkOutcome) {}

/// The deadline this park waits until: worker 0, live, parks until the
/// earliest armed timer (none armed: no deadline); every other worker, and
/// the replay park, waits for an edge only. Worker 0 reads the deadline and
/// publishes it as `parked_until` under the timers lock, BEFORE it takes
/// `wait_set` and never while holding it (§3.8.2 C2, C3).
fn park_deadline(shared: &SharedState, my_index: usize) -> Option<Instant> {
    if my_index != 0 || shared.replay_mode {
        return None;
    }
    lock_or_recover(&shared.timers).publish_park()
}

/// Worker 0 left a park it published: an arm needs no edge until the next.
fn clear_park(shared: &SharedState, my_index: usize) {
    if my_index == 0 && !shared.replay_mode {
        lock_or_recover(&shared.timers).clear_park();
    }
}

fn wait_for_edge(
    shared: &SharedState,
    my_index: usize,
    ticket: IdleTicket,
    deadline: Option<Instant>,
) -> ParkOutcome {
    if shared.shutdown.load(Ordering::Acquire) {
        return ParkOutcome::Woken;
    }
    let mut guard = lock_or_recover(&shared.wait_set);
    // Compared under the mutex every E advances under: a wake that landed
    // anywhere after the worker's sequence read returns it to its loop now.
    if shared.idle_wake.sequence.load(Ordering::Acquire) != ticket.sequence {
        return ParkOutcome::Woken;
    }
    note_wait(shared, my_index, deadline);
    #[cfg(feature = "telemetry")]
    let idle_started = Instant::now();
    let outcome = loop {
        if shared.idle_wake.sequence.load(Ordering::Acquire) != ticket.sequence
            || shared.shutdown.load(Ordering::Acquire)
        {
            break ParkOutcome::Woken;
        }
        match deadline {
            None => {
                guard = match shared.wake_condvar.wait(guard) {
                    Ok(guard) => guard,
                    Err(error) => error.into_inner(),
                };
            }
            Some(deadline) => {
                let now = Instant::now();
                if now >= deadline {
                    break ParkOutcome::TimedOut;
                }
                guard = match shared.wake_condvar.wait_timeout(guard, deadline - now) {
                    Ok((guard, _result)) => guard,
                    Err(error) => error.into_inner().0,
                };
            }
        }
    };
    note_wait_end(shared, my_index);
    drop(guard);
    #[cfg(feature = "telemetry")]
    shared.record_scheduler_idle(idle_started.elapsed());
    outcome
}
