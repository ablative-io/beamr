//! B-184: a scheduler worker that dies stops the scheduler loudly.
//!
//! Every normal worker runs under [`run_worker`], which catches a panic at
//! the top of the worker's thread. A worker that ends by a panic, or whose
//! loop returns before shutdown was asked, latches one permanent
//! [`SchedulerFailure`] naming the worker, where it was, and the panic's
//! message. The latch is never cleared and has no setting (as beamr-wasm's
//! terminal latch, WPORT-7). Latching, in order, on the dying worker's
//! thread: the value is stored; the embedder's one callback runs
//! (`Scheduler::on_failure`); shutdown is set; every parked worker and every
//! waiter is woken. Then, and as every other worker leaves its loop, the
//! worker finalizes what it can account for: its own inject queue, and every
//! process body that is not mid-slice.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crossbeam_queue::SegQueue;

use crate::process::ExitReason;

use super::execution::cleanup_exited_process;
use super::process_slot::ProcessSlot;
use super::spawning::SpawnRequest;
use super::{SharedState, lock_or_recover};

/// Where a worker was when it ended.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WorkerSite {
    /// In its idle park, about to wait or waiting on the wake condvar: the
    /// place std's condvar assert panicked on macOS.
    Parked,
    /// Running a slice of the process with this pid.
    Running(u64),
    /// In its loop between slices and parks: draining its queues, ticking
    /// timers, stealing.
    Scheduling,
}

/// How a worker ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerEnd {
    /// The worker's thread panicked, with this message.
    Panicked(String),
    /// The worker's loop returned before shutdown was asked.
    Returned,
}

/// The scheduler's permanent failure: the first worker that died, where it
/// was, and how it ended. Read with `Scheduler::failure`; returned by
/// `Scheduler::run_until_exit` once latched.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulerFailure {
    pub(super) worker: String,
    pub(super) index: usize,
    pub(super) site: WorkerSite,
    pub(super) end: WorkerEnd,
}

impl SchedulerFailure {
    /// The dead worker's thread name, `beamr-sched-N`.
    #[must_use]
    pub fn worker(&self) -> &str {
        &self.worker
    }

    /// The dead worker's index.
    #[must_use]
    pub fn worker_index(&self) -> usize {
        self.index
    }

    /// Where the worker was when it ended.
    #[must_use]
    pub fn site(&self) -> WorkerSite {
        self.site
    }

    /// How the worker ended.
    #[must_use]
    pub fn end(&self) -> &WorkerEnd {
        &self.end
    }

    /// The panic's message, or `None` when the worker's loop returned.
    #[must_use]
    pub fn panic_message(&self) -> Option<&str> {
        match &self.end {
            WorkerEnd::Panicked(message) => Some(message),
            WorkerEnd::Returned => None,
        }
    }
}

impl std::fmt::Display for SchedulerFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "scheduler worker {} died ", self.worker)?;
        match self.site {
            WorkerSite::Parked => formatter.write_str("in its idle park")?,
            WorkerSite::Running(pid) => write!(formatter, "running a slice of pid {pid}")?,
            WorkerSite::Scheduling => formatter.write_str("in its loop between slices")?,
        }
        match &self.end {
            WorkerEnd::Panicked(message) => write!(formatter, ": panicked: {message}"),
            WorkerEnd::Returned => formatter.write_str(": its loop returned before shutdown"),
        }
    }
}

impl std::error::Error for SchedulerFailure {}

type FailureCallback = Box<dyn FnOnce(&SchedulerFailure) + Send>;

enum CallbackSlot {
    /// No callback registered yet.
    Empty,
    /// Registered, waiting for the latch.
    Waiting(FailureCallback),
    /// Registered and run, or being run.
    Spent,
}

/// The permanent failure latch: one value, set once, one callback, and the
/// record of every worker that died (the value names the first).
pub(super) struct FailureLatch {
    value: OnceLock<SchedulerFailure>,
    callback: Mutex<CallbackSlot>,
    dead: Box<[AtomicBool]>,
}

impl FailureLatch {
    pub(super) fn new(worker_count: usize) -> Self {
        Self {
            value: OnceLock::new(),
            callback: Mutex::new(CallbackSlot::Empty),
            dead: (0..worker_count).map(|_| AtomicBool::new(false)).collect(),
        }
    }

    /// Whether the latch has recorded worker `index` dead.
    pub(super) fn recorded_dead(&self, index: usize) -> bool {
        self.dead
            .get(index)
            .is_some_and(|dead| dead.load(Ordering::Acquire))
    }

    pub(super) fn get(&self) -> Option<&SchedulerFailure> {
        self.value.get()
    }

    pub(super) fn is_latched(&self) -> bool {
        self.value.get().is_some()
    }

    /// Register the one callback. Runs it at once, on this thread, when the
    /// latch is already set. False, dropping `callback` unrun, when one was
    /// registered before.
    pub(super) fn register(&self, callback: FailureCallback) -> bool {
        let mut slot = lock_or_recover(&self.callback);
        if !matches!(*slot, CallbackSlot::Empty) {
            return false;
        }
        // Read under the slot lock: a latch that sets the value after this
        // read takes the slot after this registration releases it.
        let Some(failure) = self.value.get() else {
            *slot = CallbackSlot::Waiting(callback);
            return true;
        };
        *slot = CallbackSlot::Spent;
        drop(slot);
        callback(failure);
        true
    }

    /// Set the value, first death wins; then run the waiting callback, if
    /// any, on this thread. False when the latch was already set.
    fn latch(&self, failure: SchedulerFailure) -> bool {
        if let Some(dead) = self.dead.get(failure.index) {
            dead.store(true, Ordering::Release);
        }
        if self.value.set(failure).is_err() {
            return false;
        }
        let mut slot = lock_or_recover(&self.callback);
        // An empty slot stays empty: a callback registered later runs at once.
        if !matches!(*slot, CallbackSlot::Waiting(_)) {
            return true;
        }
        let taken = std::mem::replace(&mut *slot, CallbackSlot::Spent);
        drop(slot);
        if let (CallbackSlot::Waiting(callback), Some(failure)) = (taken, self.value.get()) {
            callback(failure);
        }
        true
    }
}

thread_local! {
    /// Where this worker thread is, written by its loop and its park and
    /// read by its guard after a panic, on the same thread.
    static SITE: Cell<WorkerSite> = const { Cell::new(WorkerSite::Scheduling) };
}

/// Record where this worker is (called only on worker threads).
pub(super) fn set_site(site: WorkerSite) {
    SITE.with(|cell| cell.set(site));
}

/// Each worker's report of its own thread's exit, made as its guard ends.
pub(super) struct WorkerExits {
    exited: Box<[AtomicBool]>,
    /// Held while a report is made or awaited; `changed` is notified under it.
    reports: Mutex<()>,
    changed: std::sync::Condvar,
}

impl WorkerExits {
    pub(super) fn new(worker_count: usize) -> Self {
        Self {
            exited: (0..worker_count).map(|_| AtomicBool::new(false)).collect(),
            reports: Mutex::new(()),
            changed: std::sync::Condvar::new(),
        }
    }

    fn report(&self, index: usize) {
        let held = lock_or_recover(&self.reports);
        if let Some(exited) = self.exited.get(index) {
            exited.store(true, Ordering::Release);
        }
        self.changed.notify_all();
        drop(held);
    }

    /// Block until worker `index`'s thread has reported its own exit. Ends
    /// on the report, never a clock; false at once when there is no such
    /// worker.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn await_reported(&self, index: usize) -> bool {
        let Some(exited) = self.exited.get(index) else {
            return false;
        };
        let mut held = lock_or_recover(&self.reports);
        while !exited.load(Ordering::Acquire) {
            held = match self.changed.wait(held) {
                Ok(held) => held,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
        drop(held);
        true
    }

    /// Whether worker `index`'s thread has reported its own exit.
    pub(super) fn reported(&self, index: usize) -> bool {
        self.exited
            .get(index)
            .is_some_and(|exited| exited.load(Ordering::Acquire))
    }
}

/// Why shutdown, after the latch, does not join worker `index`: the latch
/// recorded it dead, or its thread reported its own exit. `None`: join it.
pub(super) fn skip_reason(shared: &SharedState, index: usize) -> Option<&'static str> {
    if shared.failure.recorded_dead(index) {
        Some("recorded dead by the latch")
    } else if shared.worker_exits.reported(index) {
        Some("reported its own exit")
    } else {
        None
    }
}

/// Run one worker's `body` under the guard. See the module documentation.
pub(super) fn run_worker(
    shared: &SharedState,
    index: usize,
    worker: &str,
    inject: &SegQueue<SpawnRequest>,
    body: impl FnOnce(),
) {
    set_site(WorkerSite::Scheduling);
    let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    let end = match ended {
        Ok(()) if shared.shutdown.load(Ordering::Acquire) => None,
        Ok(()) => Some(WorkerEnd::Returned),
        Err(payload) => Some(WorkerEnd::Panicked(panic_text(payload.as_ref()))),
    };
    let site = SITE.with(Cell::get);
    if let Some(end) = end {
        shared.latch_failure(SchedulerFailure {
            worker: worker.to_string(),
            index,
            site,
            end,
        });
        #[cfg(any(test, feature = "test-support"))]
        super::execution::hold_if_armed(shared, index);
        // The slice this worker died in will never store its body back: the
        // body unwound with this thread. Its shadow is finalized here.
        if let WorkerSite::Running(pid) = site {
            finalize(shared, pid);
        }
    }
    if shared.failure.is_latched() {
        finalize_inject_queue(shared, inject);
        finalize_settled_bodies(shared);
    }
    shared.worker_exits.report(index);
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_string();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "a panic whose payload is not text".to_string()
}

impl SharedState {
    /// Latch `failure` (first death wins), run the embedder's callback, set
    /// shutdown, and wake every parked worker and every waiter.
    pub(super) fn latch_failure(&self, failure: SchedulerFailure) {
        if !self.failure.latch(failure) {
            return;
        }
        self.shutdown.store(true, Ordering::Release);
        self.wake_edge();
        #[cfg(any(test, feature = "test-support"))]
        super::execution::notify_park_waiters(self);
    }
}

/// The reason a finalized process is given: its tombstone, else `Killed`,
/// as shutdown gives it.
fn finalize(shared: &SharedState, pid: u64) {
    let reason = shared
        .exit_tombstones
        .get(&pid)
        .unwrap_or(ExitReason::Killed);
    cleanup_exited_process(shared, pid, reason);
}

/// Finalize every spawn left in `inject`: no worker will materialize it.
pub(super) fn finalize_inject_queue(shared: &SharedState, inject: &SegQueue<SpawnRequest>) {
    while let Some(request) = inject.pop() {
        let pid = request.pid;
        drop(request);
        finalize(shared, pid);
    }
}

/// Finalize every process body that is not mid-slice: no worker will run
/// it again. A body checked out by a worker still in its slice is left to
/// that worker, which finalizes it here as it leaves its loop.
pub(super) fn finalize_settled_bodies(shared: &SharedState) {
    let settled: Vec<u64> = shared
        .process_bodies
        .iter()
        .filter(|entry| !matches!(*lock_or_recover(entry.value()), ProcessSlot::Executing(_)))
        .map(|entry| *entry.key())
        .collect();
    for pid in settled {
        finalize(shared, pid);
    }
}
