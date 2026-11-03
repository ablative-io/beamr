//! B-184: a scheduler worker that dies stops the scheduler loudly (R2), and
//! after it every wait ends and shutdown completes (R3).
//!
//! A worker is killed through the test-support seams: at its next park
//! (`panic_worker_at_next_park`, the place std's condvar assert panicked on
//! macOS, with the `wait_set` lock held and the worker counted as parked) or
//! as it starts its next slice (`panic_worker_at_next_slice`, with work left
//! on its local run queue). `await_seam_panicked` returns once the seam has
//! fired. Every wait here ends on an event: a seam's mark, an exit or
//! completion watch, a channel, or a thread's join. None ends on a clock.

use std::sync::Arc;

use crate::atom::{Atom, AtomTable};
use crate::loader::Instruction;
use crate::module::{Module, ModuleRegistry};
use crate::native::native_process::{
    NativeContext, NativeHandler, NativeHandlerFactory, NativeOutcome,
};
use crate::process::ExitReason;

use super::tests::{deliver_atom, receive_then_return_module, test_module};
use super::{
    ExitCompletionWatch, ExitCompletionWatchState, ExitWatchState, NativeBifs, Scheduler,
    SchedulerConfig, lock_or_recover,
};

/// The worker the tests kill.
const DEAD: usize = 2;
/// A worker that stays alive.
const LIVE: usize = 3;
const WORKERS: usize = 4;

fn scheduler_with_workers(registry: &Arc<ModuleRegistry>, workers: usize) -> Arc<Scheduler> {
    Arc::new(
        Scheduler::new(
            SchedulerConfig {
                thread_count: Some(workers),
                ..SchedulerConfig::default()
            },
            Arc::clone(registry),
            NativeBifs::none(),
        )
        .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    )
}

fn return_module(registry: &ModuleRegistry, name: Atom) -> Arc<Module> {
    registry.insert(test_module(name, vec![Instruction::Return]))
}

/// Block until `pid` exits, on its exit watch.
fn await_exit(scheduler: &Scheduler, pid: u64) -> ExitReason {
    match scheduler.watch_exit(pid) {
        ExitWatchState::Live(watch) => {
            watch
                .recv()
                .unwrap_or_else(|error| panic!("exit watch for {pid}: {error}"))
                .1
        }
        ExitWatchState::AlreadyExited(reason) => reason,
        ExitWatchState::NoRecord => panic!("pid {pid} never ran"),
    }
}

fn completion_watch(scheduler: &Scheduler, pid: u64) -> ExitCompletionWatch {
    match scheduler.watch_exit_completion(pid) {
        ExitCompletionWatchState::Live(watch) => watch,
        other => panic!("pid {pid} is not live: {other:?}"),
    }
}

/// What a completion watch holds right now, read without waiting.
fn watch_answer(watch: &ExitCompletionWatch) -> String {
    match watch.try_recv() {
        Ok((pid, reason)) => format!("finalized ({pid}, {reason:?})"),
        Err(error) => format!("no answer ({error:?})"),
    }
}

/// Spawn `module` until a spawn lands on `worker` (bytecode spawns are
/// placed round-robin by `spawn_counter`), waiting for nothing. Returns that
/// spawn's pid and the pids spawned on other workers on the way.
fn spawn_on(scheduler: &Scheduler, module: &Arc<Module>, worker: usize) -> (u64, Vec<u64>) {
    let workers = scheduler.shared.thread_count;
    let mut others = Vec::new();
    loop {
        let index = scheduler
            .shared
            .spawn_counter
            .load(std::sync::atomic::Ordering::Acquire)
            % workers;
        let pid = scheduler
            .spawn_process(module)
            .unwrap_or_else(|error| panic!("spawn: {error}"));
        if index == worker {
            return (pid, others);
        }
        others.push(pid);
    }
}

/// Spawn the receive-and-return `parked` module on `worker` and block until
/// that worker has run it into its receive and waits again. Every spawn on
/// the way runs `noop` to its exit first.
fn spawn_parked_on(
    scheduler: &Scheduler,
    noop: &Arc<Module>,
    parked: &Arc<Module>,
    worker: usize,
) -> u64 {
    let workers = scheduler.shared.thread_count;
    while scheduler
        .shared
        .spawn_counter
        .load(std::sync::atomic::Ordering::Acquire)
        % workers
        != worker
    {
        let pid = scheduler
            .spawn_process(noop)
            .unwrap_or_else(|error| panic!("steering spawn: {error}"));
        assert_eq!(await_exit(scheduler, pid), ExitReason::Normal);
    }
    // A park that waits after `since` read its ticket after the spawn's
    // edge, so its checks drained the spawn and ran it first.
    let since = scheduler.worker_park_counts(worker);
    let pid = scheduler
        .spawn_process(parked)
        .unwrap_or_else(|error| panic!("spawn parked: {error}"));
    let _timeout = scheduler.await_worker_parked(worker, since);
    let parked_on = lock_or_recover(&scheduler.shared.wait_set)
        .waiting
        .get(&pid)
        .copied();
    assert_eq!(parked_on, Some(worker), "pid {pid} parks in receive on worker {worker}");
    pid
}

/// Run `scheduler.shutdown()` on its own thread and answer how that thread
/// ended: `Ok` when shutdown returned, `Err` with the panic it raised.
fn shutdown_on_another_thread(scheduler: &Arc<Scheduler>) -> Result<(), String> {
    let stopping = Arc::clone(scheduler);
    std::thread::Builder::new()
        .name("b184-shutdown".to_string())
        .spawn(move || stopping.shutdown())
        .unwrap_or_else(|error| panic!("spawn the shutdown thread: {error}"))
        .join()
        .map_err(|payload| panic_text(payload.as_ref()))
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a non-text panic payload".to_string())
}

/// What the embedder can read about the scheduler's workers.
#[derive(Debug, PartialEq)]
struct Readable {
    thread_count: usize,
    worker_names: usize,
    scheduler_count: usize,
}

fn readable(scheduler: &Scheduler) -> Readable {
    Readable {
        thread_count: scheduler.thread_count(),
        worker_names: scheduler.worker_names().len(),
        scheduler_count: scheduler.scheduler_count(),
    }
}

/// What a dead parked worker leaves behind in the shared park state.
struct LeftBehind {
    parked_workers: usize,
    idle_workers: usize,
    wait_set_poisoned: bool,
    waiting_on_dead: Vec<u64>,
    woken_for_dead: Vec<u64>,
    injected_for_dead: usize,
}

impl std::fmt::Display for LeftBehind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "parked census {}, idle count {}, wait_set poisoned {}, pids waiting on it {:?}, \
             pids woken for it {:?}, spawns in its inject queue {}",
            self.parked_workers,
            self.idle_workers,
            self.wait_set_poisoned,
            self.waiting_on_dead,
            self.woken_for_dead,
            self.injected_for_dead
        )
    }
}

fn left_behind(scheduler: &Scheduler, dead: usize) -> LeftBehind {
    let wait_set_poisoned = scheduler.shared.wait_set.is_poisoned();
    let wait_set = lock_or_recover(&scheduler.shared.wait_set);
    let mut waiting_on_dead: Vec<u64> = wait_set
        .waiting
        .iter()
        .filter(|(_, worker)| **worker == dead)
        .map(|(pid, _)| *pid)
        .collect();
    waiting_on_dead.sort_unstable();
    let woken_for_dead = wait_set
        .woken
        .iter()
        .filter(|(_, worker)| *worker == dead)
        .map(|(pid, _)| *pid)
        .collect();
    drop(wait_set);
    LeftBehind {
        parked_workers: scheduler.parked_worker_count(),
        idle_workers: scheduler.idle_worker_count(),
        wait_set_poisoned,
        waiting_on_dead,
        woken_for_dead,
        injected_for_dead: scheduler.inject_queues[dead].len(),
    }
}

/// B-184 R2's red. One worker dies at its park. Observed: whether a
/// shutdown on another thread completes, whether processes spawned after
/// the death run, whether a process parked on the dead worker runs when
/// messaged, what the embedder can read, and what the dead worker left in
/// the shared park state. Every observation is recorded before any is
/// judged.
#[test]
fn a_worker_that_dies_at_its_park_stops_the_scheduler_loudly() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let noop = return_module(&registry, atoms.intern("b184_noop"));
    let parked = receive_then_return_module(&registry, atoms.intern("b184_parked"));
    let scheduler = scheduler_with_workers(&registry, WORKERS);

    let parked_on_dead = spawn_parked_on(&scheduler, &noop, &parked, DEAD);
    let parked_watch = completion_watch(&scheduler, parked_on_dead);
    let before = readable(&scheduler);

    assert!(scheduler.panic_worker_at_next_park(DEAD));
    assert!(scheduler.await_seam_panicked(DEAD));

    // Spawned after the death: one on a live worker, one on the dead one.
    let (spawned_live, _) = spawn_on(&scheduler, &noop, LIVE);
    let (spawned_dead, others) = spawn_on(&scheduler, &noop, DEAD);
    let spawned_dead_watch = completion_watch(&scheduler, spawned_dead);
    let live_outcome = {
        let held = Arc::clone(&scheduler);
        std::thread::spawn(move || held.run_until_exit(spawned_live))
            .join()
            .map(|(reason, _)| format!("ran and exited {reason:?}"))
            .map_err(|payload| panic_text(payload.as_ref()))
    };
    // A message to the process parked on the dead worker.
    deliver_atom(&scheduler.shared, parked_on_dead, Atom::OK);
    scheduler.wake_process(parked_on_dead);
    let left = left_behind(&scheduler, DEAD);
    let after = readable(&scheduler);

    let shutdown = shutdown_on_another_thread(&scheduler);
    let parked_answer = watch_answer(&parked_watch);
    let spawned_dead_answer = watch_answer(&spawned_dead_watch);

    eprintln!(
        "B-184 R2 observations:\n  shutdown on another thread: {shutdown:?}\n  \
         spawned after the death on live worker {LIVE} (pid {spawned_live}): {live_outcome:?}\n  \
         spawned after the death on dead worker {DEAD} (pid {spawned_dead}, others {others:?}): \
         watch after shutdown {spawned_dead_answer}\n  \
         parked on dead worker {DEAD} (pid {parked_on_dead}), messaged: watch after shutdown \
         {parked_answer}\n  left behind by the dead worker: {left}\n  \
         readable before {before:?}, after {after:?}"
    );

    let mut reds = Vec::new();
    if shutdown.is_err() {
        reds.push(format!("shutdown did not return: {shutdown:?}"));
    }
    let killed = |pid: u64| format!("finalized ({pid}, {:?})", ExitReason::Killed);
    if parked_answer != killed(parked_on_dead) {
        reds.push(format!(
            "the process parked on the dead worker was not finalized by shutdown: {parked_answer}"
        ));
    }
    if spawned_dead_answer != killed(spawned_dead) {
        reds.push(format!(
            "the process spawned onto the dead worker was not finalized by shutdown: \
             {spawned_dead_answer}"
        ));
    }
    if before == after {
        reds.push(format!(
            "nothing the embedder reads changed when a worker died: {after:?}"
        ));
    }
    assert!(reds.is_empty(), "B-184 R2 red: {reds:#?}");
}

/// B-184 R3's red: a `run_until_exit` held on another thread across a
/// worker's death, for a process parked in receive on a live worker, ends,
/// and then a shutdown on another thread returns.
#[test]
fn a_run_until_exit_held_across_a_worker_death_ends_and_shutdown_returns() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let noop = return_module(&registry, atoms.intern("b184_held_noop"));
    let parked = receive_then_return_module(&registry, atoms.intern("b184_held_parked"));
    let scheduler = scheduler_with_workers(&registry, WORKERS);
    let pid = spawn_parked_on(&scheduler, &noop, &parked, LIVE);

    let held = {
        let held = Arc::clone(&scheduler);
        std::thread::spawn(move || held.run_until_exit(pid))
    };
    assert!(scheduler.panic_worker_at_next_park(DEAD));
    assert!(scheduler.await_seam_panicked(DEAD));
    let outcome = held
        .join()
        .map(|(reason, _)| format!("{reason:?}"))
        .map_err(|payload| panic_text(payload.as_ref()));
    let shutdown = shutdown_on_another_thread(&scheduler);
    eprintln!("B-184 R3 held wait: run_until_exit {outcome:?}, shutdown {shutdown:?}");
    assert!(
        shutdown.is_ok(),
        "B-184 R3 red: run_until_exit {outcome:?}, shutdown {shutdown:?}"
    );
}

/// A native process that signals once it is inside its slice, then holds
/// the worker until the test releases it, then exits normally.
struct HeldSlice {
    started: crossbeam_channel::Sender<()>,
    release: crossbeam_channel::Receiver<()>,
}

impl NativeHandler for HeldSlice {
    fn handle(&mut self, _ctx: &mut NativeContext<'_>) -> NativeOutcome {
        let _sent = self.started.send(());
        let _released = self.release.recv();
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

fn held_slice(
    started: &crossbeam_channel::Sender<()>,
    release: &crossbeam_channel::Receiver<()>,
) -> NativeHandlerFactory {
    let started = started.clone();
    let release = release.clone();
    Box::new(move || {
        Box::new(HeldSlice {
            started: started.clone(),
            release: release.clone(),
        })
    })
}

/// A native process that exits normally in its first slice.
struct ExitsAtOnce;

impl NativeHandler for ExitsAtOnce {
    fn handle(&mut self, _ctx: &mut NativeContext<'_>) -> NativeOutcome {
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

/// B-184 R3's red: one worker (so nothing is stolen) dies as it starts a
/// slice, with a second process left on its local run queue; a third is
/// spawned onto its inject queue after the death. Shutdown on another
/// thread returns, and all three are finalized by it, read from their
/// completion watches.
#[test]
fn shutdown_finalizes_the_dead_workers_local_and_inject_queues() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let noop = return_module(&registry, atoms.intern("b184_queue_noop"));
    let scheduler = scheduler_with_workers(&registry, 1);
    let (started, starts) = crossbeam_channel::unbounded();
    let (release, gate) = crossbeam_channel::unbounded();

    let gate_pid = scheduler
        .spawn_native(held_slice(&started, &gate))
        .unwrap_or_else(|error| panic!("spawn the gate: {error}"));
    starts
        .recv()
        .unwrap_or_else(|error| panic!("the gate starts: {error}"));
    // While the only worker is held: one spawn on its inject queue, one
    // native on its woken list. Its next loop moves both to its run queue.
    let injected = scheduler
        .spawn_process(&noop)
        .unwrap_or_else(|error| panic!("spawn injected: {error}"));
    let woken = scheduler
        .spawn_native(Box::new(|| Box::new(ExitsAtOnce)))
        .unwrap_or_else(|error| panic!("spawn woken: {error}"));
    let queued_watches = [
        (injected, completion_watch(&scheduler, injected)),
        (woken, completion_watch(&scheduler, woken)),
    ];
    assert!(scheduler.panic_worker_at_next_slice(0));
    release
        .send(())
        .unwrap_or_else(|error| panic!("release the gate: {error}"));
    assert_eq!(await_exit(&scheduler, gate_pid), ExitReason::Normal);
    assert!(scheduler.await_seam_panicked(0));
    let local_queue = scheduler.shared.process_bodies.len();

    let after_death = scheduler
        .spawn_process(&noop)
        .unwrap_or_else(|error| panic!("spawn after the death: {error}"));
    let after_death_watch = completion_watch(&scheduler, after_death);
    let injected_for_dead = scheduler.inject_queues[0].len();

    let shutdown = shutdown_on_another_thread(&scheduler);
    let answers: Vec<(u64, String)> = queued_watches
        .iter()
        .map(|(pid, watch)| (*pid, watch_answer(watch)))
        .chain(std::iter::once((after_death, watch_answer(&after_death_watch))))
        .collect();
    eprintln!(
        "B-184 R3 queues: bodies before shutdown {local_queue}, inject queue {injected_for_dead}, \
         shutdown {shutdown:?}, answers {answers:?}"
    );
    let mut reds = Vec::new();
    if shutdown.is_err() {
        reds.push(format!("shutdown did not return: {shutdown:?}"));
    }
    for (pid, answer) in &answers {
        if *answer != format!("finalized ({pid}, {:?})", ExitReason::Killed) {
            reds.push(format!("pid {pid} was not finalized by shutdown: {answer}"));
        }
    }
    assert!(reds.is_empty(), "B-184 R3 red: {reds:#?}");
}
