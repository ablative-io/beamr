//! B-184: a scheduler worker that dies is recovered where its state can be
//! accounted for, and otherwise stops the scheduler loudly.
//!
//! Every normal worker runs under [`run_worker`], which catches a panic at
//! the top of the worker's thread, and a loop that returns before shutdown
//! was asked. Its recovery (R5, lead amendment A) reads every
//! scheduler-shared mutex for poison and decides the tier by the event:
//!
//! - Tier 1, a worker that died parked (holding no process): when nothing
//!   but the wait set is poisoned and the wait set's invariants hold, what
//!   it left in the park is undone, the wait set's poison is cleared, and
//!   the worker restarts on this thread, its index, its local queue (the
//!   one its stealer reads) and its inject queue.
//! - Tier 2, a worker that died running a process: when nothing but that
//!   process's slot is poisoned and the invariants hold, the process is
//!   killed through the ordinary exit path (its links and monitors are
//!   told, with the reason `worker_crashed`), and the worker restarts.
//! - Tier 3, everything else, and a restarted worker that dies again before
//!   it runs one process slice: one permanent [`SchedulerFailure`] is
//!   latched, naming the worker, where it was, and the panic's message.
//!
//! Every death is counted, printed, and reported to the embedder's
//! `Scheduler::on_worker_death` callback on the dying worker's thread,
//! before it restarts or latches. The latch is never cleared and has no
//! setting (as beamr-wasm's terminal latch, WPORT-7). Latching, in order,
//! on the dying worker's thread: the value is stored; the embedder's one
//! failure callback runs (`Scheduler::on_failure`); shutdown is set; every
//! parked worker and every waiter is woken. Then, and as every other worker
//! leaves its loop, the worker finalizes what it can account for: its own
//! inject queue, and every process body that is not mid-slice.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use crossbeam_queue::SegQueue;

use crate::process::{ExitReason, ProcessStatus};

use super::ScheduledProcess;
use super::execution::cleanup_exited_process;
use super::process_slot::ProcessSlot;
use super::spawning::SpawnRequest;
use super::{SharedState, WaitSet, lock_or_recover};

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

/// What the recovery did with a dead worker (B-184 R5, lead amendment A).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DeathTier {
    /// Tier 1: the worker died parked, holding no process; the wait set was
    /// proved consistent, and the worker was restarted on its index with
    /// its queues.
    Restarted,
    /// Tier 2: the worker died running a process; that process was killed
    /// through the ordinary exit path, and the worker was restarted.
    KilledAndRestarted,
    /// Tier 3: the death latched the scheduler's permanent failure.
    Latched,
}

impl DeathTier {
    /// The tier's number, 1 to 3.
    #[must_use]
    pub fn number(self) -> u8 {
        match self {
            Self::Restarted => 1,
            Self::KilledAndRestarted => 2,
            Self::Latched => 3,
        }
    }
}

/// The recovery's poison check: every scheduler-shared mutex it read, and
/// the ones it found poisoned.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoisonCheck {
    /// Named scheduler-shared mutexes checked.
    pub mutexes: usize,
    /// Process slots checked.
    pub slots: usize,
    /// The poisoned ones, by name (`wait_set`, `link_set`, `process slot 7`).
    pub poisoned: Vec<String>,
}

impl std::fmt::Display for PoisonCheck {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} mutexes and {} process slots checked, poisoned [{}]",
            self.mutexes,
            self.slots,
            self.poisoned.join(", ")
        )
    }
}

/// The recovery's wait-set check: the entries it read and the violations
/// of the wait set's invariants it found (every waiting pid names a live
/// process waiting or suspended; every woken entry names a live pid).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WaitSetCheck {
    /// `waiting` entries checked.
    pub waiting: usize,
    /// `woken` entries checked.
    pub woken: usize,
    /// Each violation found, named.
    pub violations: Vec<String>,
}

impl WaitSetCheck {
    /// Whether the invariants held.
    #[must_use]
    pub fn holds(&self) -> bool {
        self.violations.is_empty()
    }
}

impl std::fmt::Display for WaitSetCheck {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} waiting and {} woken entries checked, {} violations",
            self.waiting,
            self.woken,
            self.violations.len()
        )?;
        if !self.violations.is_empty() {
            write!(formatter, " [{}]", self.violations.join("; "))?;
        }
        Ok(())
    }
}

/// One worker's death, of any tier, as reported to `Scheduler::on_worker_death`
/// and printed by the recovery (B-184 R5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerDeath {
    pub(super) worker: String,
    pub(super) index: usize,
    pub(super) site: WorkerSite,
    pub(super) end: WorkerEnd,
    pub(super) tier: DeathTier,
    pub(super) crashed_pid: Option<u64>,
    pub(super) poison: PoisonCheck,
    pub(super) wait_set: Option<WaitSetCheck>,
    pub(super) cause: String,
}

impl WorkerDeath {
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

    /// Where the worker was when it died.
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

    /// What the recovery did.
    #[must_use]
    pub fn tier(&self) -> DeathTier {
        self.tier
    }

    /// The process killed as a crash (tier 2 only).
    #[must_use]
    pub fn crashed_pid(&self) -> Option<u64> {
        self.crashed_pid
    }

    /// The poison check's figure.
    #[must_use]
    pub fn poison_check(&self) -> &PoisonCheck {
        &self.poison
    }

    /// The wait-set check's figure, `None` where the tier was decided
    /// without it.
    #[must_use]
    pub fn wait_set_check(&self) -> Option<&WaitSetCheck> {
        self.wait_set.as_ref()
    }

    /// Why the recovery chose this tier.
    #[must_use]
    pub fn cause(&self) -> &str {
        &self.cause
    }
}

impl std::fmt::Display for WorkerDeath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "scheduler worker {} died ", self.worker)?;
        match self.site {
            WorkerSite::Parked => formatter.write_str("in its idle park")?,
            WorkerSite::Running(pid) => write!(formatter, "running a slice of pid {pid}")?,
            WorkerSite::Scheduling => formatter.write_str("in its loop between slices")?,
        }
        match &self.end {
            WorkerEnd::Panicked(message) => write!(formatter, ": panicked: {message}")?,
            WorkerEnd::Returned => formatter.write_str(": its loop returned before shutdown")?,
        }
        write!(formatter, "; poison check: {}", self.poison)?;
        match &self.wait_set {
            Some(check) => write!(formatter, "; wait set check: {check}")?,
            None => formatter.write_str("; wait set check: not made")?,
        }
        write!(formatter, "; tier {}: ", self.tier.number())?;
        match (self.tier, self.crashed_pid) {
            (DeathTier::Restarted, _) => write!(
                formatter,
                "restarted on index {} with its queues",
                self.index
            )?,
            (DeathTier::KilledAndRestarted, Some(pid)) => write!(
                formatter,
                "pid {pid} killed as a crash, restarted on index {} with its queues",
                self.index
            )?,
            (DeathTier::KilledAndRestarted, None) => write!(
                formatter,
                "restarted on index {} with its queues",
                self.index
            )?,
            (DeathTier::Latched, _) => formatter.write_str("the scheduler's failure is latched")?,
        }
        write!(formatter, " ({})", self.cause)
    }
}

type DeathCallback = Box<dyn Fn(&WorkerDeath) + Send + Sync>;

/// Every death of every tier: the count, and the embedder's one callback.
pub(super) struct WorkerDeaths {
    count: AtomicU64,
    callback: OnceLock<DeathCallback>,
}

impl WorkerDeaths {
    fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            callback: OnceLock::new(),
        }
    }

    /// Register the one death callback: false, dropping `callback`, when
    /// one was registered before. A death is reported only to a callback
    /// registered before it; the count covers every death.
    pub(super) fn register(&self, callback: DeathCallback) -> bool {
        self.callback.set(callback).is_ok()
    }

    pub(super) fn count(&self) -> u64 {
        self.count.load(Ordering::Acquire)
    }

    /// Count `death`, print it, and run the callback on this thread.
    fn report(&self, death: &WorkerDeath) {
        self.count.fetch_add(1, Ordering::AcqRel);
        eprintln!("beamr: {death}");
        if let Some(callback) = self.callback.get() {
            callback(death);
        }
    }
}

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
    /// B-184 R5: every death, of every tier.
    pub(super) deaths: WorkerDeaths,
}

impl FailureLatch {
    pub(super) fn new(worker_count: usize) -> Self {
        Self {
            value: OnceLock::new(),
            callback: Mutex::new(CallbackSlot::Empty),
            dead: (0..worker_count).map(|_| AtomicBool::new(false)).collect(),
            deaths: WorkerDeaths::new(),
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
    /// Whether this worker has run a process slice since it last started or
    /// was restarted (B-184 R5), written by its loop.
    static SLICE_RUN: Cell<bool> = const { Cell::new(false) };
}

/// Record where this worker is (called only on worker threads).
pub(super) fn set_site(site: WorkerSite) {
    SITE.with(|cell| cell.set(site));
}

/// Record that this worker ran a process slice (called only on worker
/// threads, as each slice returns).
pub(super) fn note_slice_run() {
    SLICE_RUN.with(|cell| cell.set(true));
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

/// Run one worker's `body`, its scheduler loop, under the guard; after a
/// death the recovery decides the tier, and a restart runs `body` again on
/// this thread, with the same queues the closure holds. See the module
/// documentation.
pub(super) fn run_worker(
    shared: &SharedState,
    index: usize,
    worker: &str,
    inject: &SegQueue<SpawnRequest>,
    mut body: impl FnMut(),
) {
    let mut restarted = false;
    loop {
        set_site(WorkerSite::Scheduling);
        SLICE_RUN.with(|cell| cell.set(false));
        let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut body));
        let end = match ended {
            Ok(()) if shared.shutdown.load(Ordering::Acquire) => None,
            Ok(()) => Some(WorkerEnd::Returned),
            Err(payload) => Some(WorkerEnd::Panicked(panic_text(payload.as_ref()))),
        };
        let Some(end) = end else {
            break;
        };
        let death = Death {
            index,
            worker,
            site: SITE.with(Cell::get),
            end,
            before_a_slice: restarted && !SLICE_RUN.with(Cell::get),
        };
        if recover(shared, death) == AfterDeath::Stop {
            break;
        }
        restarted = true;
    }
    if shared.failure.is_latched() {
        finalize_inject_queue(shared, inject);
        finalize_settled_bodies(shared);
    }
    shared.worker_exits.report(index);
}

/// What the guard caught.
struct Death<'a> {
    index: usize,
    worker: &'a str,
    site: WorkerSite,
    end: WorkerEnd,
    /// The worker had been restarted and ran no process slice since.
    before_a_slice: bool,
}

/// What the guard does after the recovery.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum AfterDeath {
    /// Run the scheduler loop again on this thread.
    Restart,
    /// Leave: the failure is latched.
    Stop,
}

/// The cause a restarted worker's second death before a slice is given.
const DIED_AGAIN: &str = "it died again after its restart before it ran one process slice";

/// B-184 R5: decide the tier of `death` from the poison check and the
/// wait-set check, undo what it left (tier 1) or kill the process it was
/// running (tier 2), report it, and restart or latch (tier 3).
fn recover(shared: &SharedState, death: Death<'_>) -> AfterDeath {
    #[cfg(test)]
    super::execution::hold_before_recovery_if_armed(shared, death.index);
    let poison = poison_check(shared);
    let (tier, wait_set, cause) = decide(shared, &death, &poison);
    let crashed_pid = match (tier, death.site) {
        (DeathTier::KilledAndRestarted, WorkerSite::Running(pid)) => Some(pid),
        _ => None,
    };
    match tier {
        DeathTier::Restarted => {
            // The check held, and cleared the wait set's poison over the
            // state it proved: what the dead worker left in the park is undone.
            super::execution::undo_dead_park(shared, death.index);
        }
        DeathTier::KilledAndRestarted => {
            if let Some(pid) = crashed_pid
                && !shared.exit_tombstones.contains_key(&pid)
            {
                cleanup_exited_process(shared, pid, ExitReason::WorkerCrashed);
            }
        }
        DeathTier::Latched => {}
    }
    shared.failure.deaths.report(&WorkerDeath {
        worker: death.worker.to_string(),
        index: death.index,
        site: death.site,
        end: death.end.clone(),
        tier,
        crashed_pid,
        poison,
        wait_set,
        cause,
    });
    if tier != DeathTier::Latched {
        return AfterDeath::Restart;
    }
    shared.latch_failure(SchedulerFailure {
        worker: death.worker.to_string(),
        index: death.index,
        site: death.site,
        end: death.end,
    });
    #[cfg(any(test, feature = "test-support"))]
    super::execution::hold_if_armed(shared, death.index);
    // The slice this worker died in will never store its body back: the
    // body unwound with this thread. Its shadow is finalized here.
    if let WorkerSite::Running(pid) = death.site {
        finalize(shared, pid);
    }
    AfterDeath::Stop
}

/// The tier, the wait-set check where it was made, and why. No count and
/// no timer decide it: the death's site, the poison found and the check do.
fn decide(
    shared: &SharedState,
    death: &Death<'_>,
    poison: &PoisonCheck,
) -> (DeathTier, Option<WaitSetCheck>, String) {
    let latched = |cause: &str| (DeathTier::Latched, None, cause.to_string());
    if shared.failure.is_latched() {
        return latched("the scheduler's failure was already latched");
    }
    if death.end == WorkerEnd::Returned {
        return latched("its loop returned before shutdown");
    }
    if death.before_a_slice {
        return latched(DIED_AGAIN);
    }
    // The one mutex each recoverable site may leave poisoned: the wait set
    // the park held, or the slot of the process the worker was running.
    let (tier, own) = match death.site {
        WorkerSite::Parked => (DeathTier::Restarted, "wait_set".to_string()),
        WorkerSite::Running(pid) => (DeathTier::KilledAndRestarted, slot_name(pid)),
        WorkerSite::Scheduling => {
            return latched(
                "it died in its loop between slices, where what it held cannot be accounted for",
            );
        }
    };
    let unproven: Vec<&str> = poison
        .poisoned
        .iter()
        .map(String::as_str)
        .filter(|name| *name != own)
        .collect();
    if !unproven.is_empty() {
        return latched(&format!(
            "poisoned, and no check proves them consistent: {}",
            unproven.join(", ")
        ));
    }
    let check = wait_set_check(shared, tier == DeathTier::Restarted);
    if !check.holds() {
        return (
            DeathTier::Latched,
            Some(check),
            "the wait set check failed".to_string(),
        );
    }
    let cause = match tier {
        DeathTier::Restarted => {
            "it died parked, holding no process, and the wait set is consistent"
        }
        _ => "it died running a process, killed as a crash, and nothing else is poisoned",
    };
    (tier, Some(check), cause.to_string())
}

fn slot_name(pid: u64) -> String {
    format!("process slot {pid}")
}

/// Read every scheduler-shared mutex the shared state holds, and every
/// process slot, for poison. Lock-free: `is_poisoned` takes no lock.
fn poison_check(shared: &SharedState) -> PoisonCheck {
    let mut named: Vec<(&str, bool)> = vec![
        ("wait_set", shared.wait_set.is_poisoned()),
        ("link_set", shared.link_set.is_poisoned()),
        ("monitor_set", shared.monitor_set.is_poisoned()),
        ("timers", shared.timers.is_poisoned()),
        ("output_sink", shared.output_sink.is_poisoned()),
        ("io_bridge", shared.io_bridge.is_poisoned()),
        ("dirty_completions", shared.dirty_completions.is_poisoned()),
        (
            "dirty_completion_shutdown_tx",
            shared.dirty_completion_shutdown_tx.is_poisoned(),
        ),
        ("shutdown_skipped", shared.shutdown_skipped.is_poisoned()),
        (
            "exit_completion_watches",
            shared.exit_completion_watches.is_poisoned(),
        ),
        ("exit_tombstones", shared.exit_tombstones.is_poisoned()),
        ("pg_registry", shared.pg_registry.is_poisoned()),
        ("failure callback", shared.failure.callback.is_poisoned()),
        (
            "worker exit reports",
            shared.worker_exits.reports.is_poisoned(),
        ),
        (
            "replay_driver",
            shared
                .replay_driver
                .as_ref()
                .is_some_and(|driver| driver.is_poisoned()),
        ),
    ];
    named.extend(telemetry_mutexes(shared));
    let mut poisoned: Vec<String> = named
        .iter()
        .filter(|(_, poisoned)| *poisoned)
        .map(|(name, _)| (*name).to_string())
        .collect();
    let mut slots = 0;
    for entry in &shared.process_bodies {
        slots += 1;
        if entry.value().is_poisoned() {
            poisoned.push(slot_name(*entry.key()));
        }
    }
    PoisonCheck {
        mutexes: named.len(),
        slots,
        poisoned,
    }
}

/// Check the wait set's invariants: every waiting pid names a live process
/// that is waiting or suspended; every woken entry names a live pid. The
/// entries are read under the `wait_set` lock, and each process is read
/// after it is released (never a slot lock under `wait_set`). Then, under
/// the lock again, the entries are compared with what was read: when they
/// changed meanwhile the check runs again over the new entries; when they
/// are unchanged the result stands, and (tier 1, `clear_poison`) the wait
/// set's poison is cleared inside that same hold, only when the check held,
/// so the clear covers exactly the state the check proved (lead amendment
/// A.1, F2). A worker that panics holding `wait_set` after that hold poisons
/// it again for its own recovery to read. Ends when one pass reads entries
/// that did not change under it; no count and no timer.
fn wait_set_check(shared: &SharedState, clear_poison: bool) -> WaitSetCheck {
    loop {
        let read = WaitSetEntries::read(&lock_or_recover(&shared.wait_set));
        let mut violations = Vec::new();
        for (pid, worker) in &read.waiting {
            if let Some(why) = waiting_violation(shared, *pid) {
                violations.push(format!("waiting pid {pid} on worker {worker}: {why}"));
            }
        }
        for (pid, worker) in &read.woken {
            if shared.process_table.get(*pid).is_none() {
                violations.push(format!(
                    "woken pid {pid} for worker {worker}: no live process"
                ));
            }
        }
        let held = lock_or_recover(&shared.wait_set);
        if WaitSetEntries::read(&held) != read {
            continue;
        }
        if clear_poison && violations.is_empty() {
            shared.wait_set.clear_poison();
        }
        drop(held);
        return WaitSetCheck {
            waiting: read.waiting.len(),
            woken: read.woken.len(),
            violations,
        };
    }
}

/// The wait set's entries as read under its lock, `waiting` sorted so two
/// reads of the same entries compare equal.
#[derive(Eq, PartialEq)]
struct WaitSetEntries {
    waiting: Vec<(u64, usize)>,
    woken: Vec<(u64, usize)>,
}

impl WaitSetEntries {
    fn read(wait_set: &WaitSet) -> Self {
        let mut waiting: Vec<(u64, usize)> = wait_set
            .waiting
            .iter()
            .map(|(pid, worker)| (*pid, *worker))
            .collect();
        waiting.sort_unstable();
        Self {
            waiting,
            woken: wait_set.woken.clone(),
        }
    }
}

/// Why waiting `pid` breaks the invariant, or `None`.
fn waiting_violation(shared: &SharedState, pid: u64) -> Option<String> {
    if shared.process_table.get(pid).is_none() {
        return Some("no live process".to_string());
    }
    let Some(entry) = shared.process_bodies.get(&pid) else {
        return Some("no process body".to_string());
    };
    let slot = lock_or_recover(entry.value());
    match &*slot {
        ProcessSlot::Present(ScheduledProcess(process)) => match process.status() {
            ProcessStatus::Waiting | ProcessStatus::Suspended => None,
            other => Some(format!(
                "its process is {other:?}, not waiting or suspended"
            )),
        },
        ProcessSlot::Executing(_) => Some("its process is mid-slice".to_string()),
        ProcessSlot::Absent => Some("its process body is absent".to_string()),
    }
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

/// The telemetry state's mutex, for the poison check.
#[cfg(feature = "telemetry")]
fn telemetry_mutexes(shared: &SharedState) -> Option<(&'static str, bool)> {
    Some((
        "last_process_samples",
        shared.telemetry_metrics.last_process_samples.is_poisoned(),
    ))
}

/// No telemetry state without the feature.
#[cfg(not(feature = "telemetry"))]
fn telemetry_mutexes(_shared: &SharedState) -> Option<(&'static str, bool)> {
    None
}
