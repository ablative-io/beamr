//! The idle park of a normal scheduler worker, and its test instruments.
//!
//! EMBEDDER-COMPOSITION-SPEC §3.8 (B-181). An idle worker parks with no
//! timeout; worker 0 parks only until the earliest armed timer, the one
//! source it must reach by the clock. Every other source runs edge E (take
//! the `wait_set` lock, advance the wake sequence, `notify_all`), and the
//! park waits only while, under that lock, the sequence is the one the
//! worker read before its checks — so no wake is lost and none is polled.
//!
//! The seams the tickless-idle tests read are in `seams`.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering, fence};
use std::time::Instant;

use crate::scheduler::{RunQueue, SharedState, WaitSet, lock_or_recover};

mod seams;
#[cfg(test)]
pub(in crate::scheduler) use seams::ParkHooks;
pub(super) use seams::account_deadline_wake;
#[cfg(any(test, feature = "test-support"))]
pub use seams::{ObservedParkTimeout, WakeCauses, WorkerParkCounts};
#[cfg(any(test, feature = "test-support"))]
pub(in crate::scheduler) use seams::{ParkSeams, await_all_parked, await_worker_parked};
use seams::{count_entry, note_wait, note_wait_end, record_outcome};
#[cfg(test)]
pub(super) use seams::{idle_gap, report_left_park};

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
