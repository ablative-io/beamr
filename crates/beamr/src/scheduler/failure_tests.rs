//! B-184: a scheduler worker that dies stops the scheduler loudly (R2), and
//! after it every wait ends and shutdown completes (R3).
//!
//! A worker is killed through the test-support seams: at its next park
//! (`panic_worker_at_next_park`, the place std's condvar assert panicked on
//! macOS, with the `wait_set` lock held and the worker counted as parked) or
//! as it starts its next slice (`panic_worker_at_next_slice`), or by a native
//! handler that panics inside its own slice. `await_seam_panicked` returns
//! once a seam has fired; `hold_worker_after_death` keeps a dead worker's
//! thread wedged inside its guard. Every wait here ends on an event: a seam's
//! mark, the failure callback, an exit or completion watch, a channel, a
//! worker's exit report, or a thread's join. None ends on a clock.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::atom::{Atom, AtomTable};
use crate::error::ExecError;
use crate::loader::Instruction;
use crate::module::{Module, ModuleRegistry};
use crate::native::actor::{Actor, ActorContext, ActorError, spawn_actor};
use crate::native::native_process::{
    NativeContext, NativeHandler, NativeHandlerFactory, NativeOutcome,
};
use crate::process::ExitReason;

use super::tests::{receive_then_return_module, test_module};
use super::{
    ExitCompletionWatch, ExitCompletionWatchState, ExitEvent, ExitWatchState, NativeBifs,
    Scheduler, SchedulerConfig, SchedulerFailure, WorkerEnd, WorkerSite, lock_or_recover,
    park_seam_panic_message,
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

/// The failure a park seam latches for worker `index`.
fn park_failure(index: usize) -> SchedulerFailure {
    SchedulerFailure {
        worker: format!("beamr-sched-{index}"),
        index,
        site: WorkerSite::Parked,
        end: WorkerEnd::Panicked(park_seam_panic_message(index)),
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
    while scheduler.shared.spawn_counter.load(Ordering::Acquire) % workers != worker {
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
    assert_eq!(
        parked_on,
        Some(worker),
        "pid {pid} parks in receive on worker {worker}"
    );
    pid
}

/// Run `scheduler.shutdown()` on its own thread and answer how that thread
/// ended, observed by its join: `Ok` when shutdown returned.
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

/// What the failure callback saw: the thread it ran on, the failure, and
/// whether shutdown was already set when it ran.
type Called = (Option<String>, SchedulerFailure, bool);

/// Register a callback that reports what it saw on the returned channel.
fn report_failure(scheduler: &Arc<Scheduler>) -> crossbeam_channel::Receiver<Called> {
    let (called, calls) = crossbeam_channel::unbounded();
    let observed = Arc::clone(&scheduler.shared);
    assert!(scheduler.on_failure(move |failure| {
        let thread = std::thread::current().name().map(str::to_string);
        let shutdown_set = observed.shutdown.load(Ordering::Acquire);
        let _sent = called.send((thread, failure.clone(), shutdown_set));
    }));
    calls
}

/// Register a callback that reports what it saw, then holds the dying
/// worker inside its latch, before shutdown is set, until `release` is
/// dropped: so a second worker armed to die reaches its park.
fn report_failure_and_hold(
    scheduler: &Arc<Scheduler>,
) -> (
    crossbeam_channel::Receiver<Called>,
    crossbeam_channel::Sender<()>,
) {
    let (called, calls) = crossbeam_channel::unbounded();
    let (release, held) = crossbeam_channel::bounded::<()>(0);
    let observed = Arc::clone(&scheduler.shared);
    assert!(scheduler.on_failure(move |failure| {
        let thread = std::thread::current().name().map(str::to_string);
        let shutdown_set = observed.shutdown.load(Ordering::Acquire);
        let _sent = called.send((thread, failure.clone(), shutdown_set));
        let _released = held.recv();
    }));
    (calls, release)
}

/// Shutdown's skip list holds exactly the `dead` workers as recorded dead,
/// and otherwise only workers that had reported their own exit.
fn assert_skipped(skipped: &[String], dead: &[usize]) {
    for index in dead {
        let entry = format!("beamr-sched-{index} (recorded dead by the latch)");
        assert!(skipped.contains(&entry), "{entry} in {skipped:?}");
    }
    let others = skipped
        .iter()
        .filter(|entry| !entry.ends_with("(recorded dead by the latch)"));
    for entry in others {
        assert!(
            entry.ends_with("(reported its own exit)"),
            "only exited workers are skipped besides the dead: {skipped:?}"
        );
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

/// B-184 R2 (red at e3ecebd8, 9560fc91). One worker dies at its park. The
/// failure is latched with the worker's name, its site and the panic's
/// message, read by value; the callback registered before the death ran on
/// the dying worker's thread before shutdown was set; a spawn after the
/// death is refused; a `run_until_exit` and a shutdown, each on its own
/// thread, return; and the process parked on the dead worker is finalized.
#[test]
fn a_worker_that_dies_at_its_park_stops_the_scheduler_loudly() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let noop = return_module(&registry, atoms.intern("b184_noop"));
    let parked = receive_then_return_module(&registry, atoms.intern("b184_parked"));
    let scheduler = scheduler_with_workers(&registry, WORKERS);
    let calls = report_failure(&scheduler);

    let parked_on_dead = spawn_parked_on(&scheduler, &noop, &parked, DEAD);
    let parked_watch = completion_watch(&scheduler, parked_on_dead);
    assert_eq!(
        scheduler.failure(),
        None,
        "no failure while every worker runs"
    );

    assert!(scheduler.panic_worker_at_next_park(DEAD));
    assert!(scheduler.await_seam_panicked(DEAD));
    let (thread, called_with, shutdown_set) = calls
        .recv()
        .unwrap_or_else(|error| panic!("the failure callback runs: {error}"));
    let left = left_behind(&scheduler, DEAD);

    let spawned_after = scheduler.spawn_process(&noop);
    let held = {
        let held = Arc::clone(&scheduler);
        std::thread::spawn(move || held.run_until_exit(parked_on_dead))
            .join()
            .map(|outcome| outcome.map(|(reason, _)| reason))
            .map_err(|payload| panic_text(payload.as_ref()))
    };
    let shutdown = shutdown_on_another_thread(&scheduler);
    let parked_answer = parked_watch.recv();
    eprintln!(
        "B-184 R2 green: failure {:?}\n  callback on {thread:?}, shutdown set when it ran: \
         {shutdown_set}\n  spawn after the death: {spawned_after:?}\n  run_until_exit: \
         {held:?}\n  shutdown: {shutdown:?}, skipped {:?}\n  parked on the dead worker: \
         {parked_answer:?}\n  left behind by the dead worker: {left}",
        scheduler.failure(),
        scheduler.shutdown_skipped_workers()
    );

    let expected = park_failure(DEAD);
    assert_eq!(scheduler.failure(), Some(expected.clone()));
    assert_eq!(expected.worker(), "beamr-sched-2");
    assert_eq!(
        expected.panic_message(),
        Some(
            "B-184 park seam: beamr-sched-2 panicked in its park, where std's condvar \
             assert ended the workers on macOS"
        )
    );
    assert_eq!(called_with, expected);
    assert_eq!(thread.as_deref(), Some("beamr-sched-2"));
    assert!(
        !shutdown_set,
        "the callback runs before the latch sets shutdown"
    );
    assert_eq!(spawned_after, Err(ExecError::SchedulerTerminated));
    assert_eq!(held, Ok(Err(expected)));
    assert_eq!(shutdown, Ok(()));
    assert_eq!(parked_answer, Ok((parked_on_dead, ExitReason::Killed)));
}

/// B-184 R2: two workers panicked at once latch one failure, naming the
/// first, and the callback runs exactly once. Both deaths are recorded,
/// and shutdown skips both.
#[test]
fn two_workers_dying_at_once_latch_one_failure_and_call_back_once() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = scheduler_with_workers(&registry, WORKERS);
    let (calls, release) = report_failure_and_hold(&scheduler);

    assert!(scheduler.panic_worker_at_next_park(1));
    assert!(scheduler.panic_worker_at_next_park(DEAD));
    assert!(scheduler.await_seam_panicked(1));
    assert!(scheduler.await_seam_panicked(DEAD));
    drop(release);
    assert!(scheduler.await_worker_exit_reported(1));
    assert!(scheduler.await_worker_exit_reported(DEAD));
    let (thread, called_with, _) = calls
        .recv()
        .unwrap_or_else(|error| panic!("the failure callback runs: {error}"));
    let further_calls = calls.try_iter().count();
    let shutdown = shutdown_on_another_thread(&scheduler);

    let latched = scheduler
        .failure()
        .unwrap_or_else(|| panic!("a failure is latched"));
    assert!(
        latched == park_failure(1) || latched == park_failure(DEAD),
        "the latched failure names one of the two: {latched:?}"
    );
    assert_eq!(called_with, latched, "the callback saw the latched failure");
    assert_eq!(thread.as_deref(), Some(latched.worker()));
    assert_eq!(further_calls, 0, "the callback ran exactly once");
    assert_eq!(shutdown, Ok(()));
    assert_skipped(&scheduler.shutdown_skipped_workers(), &[1, DEAD]);
}

/// B-184 lead amendment B: a callback registered after the latch runs at
/// once on the registering thread, and a second registration is refused.
#[test]
fn a_callback_registered_after_the_latch_runs_at_once_on_the_registering_thread() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = scheduler_with_workers(&registry, WORKERS);
    assert!(scheduler.panic_worker_at_next_park(DEAD));
    assert!(scheduler.await_seam_panicked(DEAD));
    assert!(scheduler.await_worker_exit_reported(DEAD));

    let (called, calls) = crossbeam_channel::unbounded();
    let registering = std::thread::current().id();
    assert!(scheduler.on_failure(move |failure| {
        let _sent = called.send((std::thread::current().id(), failure.clone()));
    }));
    let (ran_on, failure) = calls
        .try_recv()
        .unwrap_or_else(|error| panic!("the callback ran before on_failure returned: {error}"));
    assert_eq!(ran_on, registering);
    assert_eq!(failure, park_failure(DEAD));
    assert!(
        !scheduler.on_failure(|_| {}),
        "a second callback is refused"
    );
    assert_eq!(shutdown_on_another_thread(&scheduler), Ok(()));
}

/// B-184 R3 (red at e3ecebd8, 9560fc91): a `run_until_exit` held on
/// another thread across a worker's death, for a process parked in receive
/// on a live worker, returns the failure, and then a shutdown on another
/// thread returns, each observed by its own join.
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
        .map(|outcome| outcome.map(|(reason, _)| reason))
        .map_err(|payload| panic_text(payload.as_ref()));
    let shutdown = shutdown_on_another_thread(&scheduler);
    assert_eq!(outcome, Ok(Err(park_failure(DEAD))));
    assert_eq!(shutdown, Ok(()));
}

/// A native process that signals once it is inside its slice, holds the
/// worker until the test releases it, then exits normally, or panics.
struct HeldSlice {
    started: crossbeam_channel::Sender<u64>,
    release: crossbeam_channel::Receiver<()>,
    panics: bool,
}

/// The message a [`HeldSlice`] that panics panics with.
const HANDLER_PANIC: &str = "B-184 test: a native handler panicked inside its slice";

impl NativeHandler for HeldSlice {
    fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
        let _sent = self.started.send(ctx.self_pid());
        let _released = self.release.recv();
        assert!(!self.panics, "{HANDLER_PANIC}");
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

fn held_slice(
    started: &crossbeam_channel::Sender<u64>,
    release: &crossbeam_channel::Receiver<()>,
    panics: bool,
) -> NativeHandlerFactory {
    let started = started.clone();
    let release = release.clone();
    Box::new(move || {
        Box::new(HeldSlice {
            started: started.clone(),
            release: release.clone(),
            panics,
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

/// B-184 R3 (red at e3ecebd8, 9560fc91, there with the slice seam). One
/// worker, so nothing is stolen and its run queue is FIFO. A gate holds it
/// while a panicking native and a second native are spawned onto its woken
/// list; its next loop moves both to its local run queue and runs the
/// panicking one, which holds the worker while a bytecode spawn lands on its
/// inject queue, then panics inside its slice. Shutdown on another thread
/// returns, and the process it died running, the one on its local queue and
/// the one on its inject queue are all finalized, read from their watches.
#[test]
fn shutdown_finalizes_the_dead_workers_local_and_inject_queues() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let noop = return_module(&registry, atoms.intern("b184_queue_noop"));
    let scheduler = scheduler_with_workers(&registry, 1);
    let (started, starts) = crossbeam_channel::unbounded();
    let (release, gate) = crossbeam_channel::unbounded();
    let calls = report_failure(&scheduler);

    let gate_pid = scheduler
        .spawn_native(held_slice(&started, &gate, false))
        .unwrap_or_else(|error| panic!("spawn the gate: {error}"));
    assert_eq!(starts.recv(), Ok(gate_pid));
    let panicking = scheduler
        .spawn_native(held_slice(&started, &gate, true))
        .unwrap_or_else(|error| panic!("spawn the panicking native: {error}"));
    let local = scheduler
        .spawn_native(Box::new(|| Box::new(ExitsAtOnce)))
        .unwrap_or_else(|error| panic!("spawn the local native: {error}"));
    let watches = [
        (panicking, completion_watch(&scheduler, panicking)),
        (local, completion_watch(&scheduler, local)),
    ];
    release
        .send(())
        .unwrap_or_else(|error| panic!("release the gate: {error}"));
    assert_eq!(await_exit(&scheduler, gate_pid), ExitReason::Normal);
    assert_eq!(starts.recv(), Ok(panicking));
    let injected = scheduler
        .spawn_process(&noop)
        .unwrap_or_else(|error| panic!("spawn onto the inject queue: {error}"));
    let injected_watch = completion_watch(&scheduler, injected);
    let queued = (
        scheduler.shared.process_bodies.contains_key(&local),
        scheduler.inject_queues[0].len(),
    );
    release
        .send(())
        .unwrap_or_else(|error| panic!("release the panicking native: {error}"));
    let (_, failure, _) = calls
        .recv()
        .unwrap_or_else(|error| panic!("the failure callback runs: {error}"));

    let shutdown = shutdown_on_another_thread(&scheduler);
    let answers: Vec<_> = watches
        .iter()
        .map(|(pid, watch)| (*pid, watch.recv()))
        .chain(std::iter::once((injected, injected_watch.recv())))
        .collect();
    eprintln!("B-184 R3 queues: queued {queued:?}, failure {failure}, answers {answers:?}");
    assert_eq!(queued, (true, 1), "one on the local queue, one injected");
    assert_eq!(
        failure,
        SchedulerFailure {
            worker: "beamr-sched-0".to_string(),
            index: 0,
            site: WorkerSite::Running(panicking),
            end: WorkerEnd::Panicked(HANDLER_PANIC.to_string()),
        }
    );
    assert_eq!(shutdown, Ok(()));
    for (pid, answer) in answers {
        assert_eq!(
            answer,
            Ok((pid, ExitReason::Killed)),
            "pid {pid} is finalized"
        );
    }
}

/// B-184 R3 (lead amendment B): a worker held wedged by the test-support
/// hold after the latch recorded it dead, and a worker recorded dead whose
/// thread ended, hold up neither shutdown nor drop. Each returns, observed
/// by its own thread's join, and shutdown names the skipped workers.
#[test]
fn a_wedged_dead_worker_holds_up_neither_shutdown_nor_drop() {
    let registry = Arc::new(ModuleRegistry::new());

    let scheduler = scheduler_with_workers(&registry, WORKERS);
    let (calls, release) = report_failure_and_hold(&scheduler);
    let hold = scheduler
        .hold_worker_after_death(1)
        .unwrap_or_else(|| panic!("worker 1 exists"));
    assert!(scheduler.panic_worker_at_next_park(1));
    assert!(scheduler.await_seam_panicked(1));
    let (first, _, _) = calls
        .recv()
        .unwrap_or_else(|error| panic!("the failure callback runs: {error}"));
    assert!(scheduler.panic_worker_at_next_park(DEAD));
    assert!(scheduler.await_seam_panicked(DEAD));
    drop(release);
    assert!(scheduler.await_worker_exit_reported(DEAD));
    let shutdown = shutdown_on_another_thread(&scheduler);
    let skipped = scheduler.shutdown_skipped_workers();
    hold.release();

    let dropped = scheduler_with_workers(&registry, WORKERS);
    let dropped_calls = report_failure(&dropped);
    let drop_hold = dropped
        .hold_worker_after_death(1)
        .unwrap_or_else(|| panic!("worker 1 exists"));
    assert!(dropped.panic_worker_at_next_park(1));
    assert!(dropped.await_seam_panicked(1));
    let (_, failed, _) = dropped_calls
        .recv()
        .unwrap_or_else(|error| panic!("the failure callback runs: {error}"));
    assert_eq!(dropped.failure(), Some(failed.clone()));
    let dropping = std::thread::spawn(move || drop(dropped))
        .join()
        .map_err(|payload| panic_text(payload.as_ref()));
    drop(drop_hold);

    assert_eq!(first.as_deref(), Some("beamr-sched-1"));
    assert_eq!(shutdown, Ok(()));
    assert_skipped(&skipped, &[1, DEAD]);
    assert_eq!(failed, park_failure(1));
    assert_eq!(dropping, Ok(()), "drop returns with the dead worker wedged");
}

/// B-184 R3: every wait an embedder holds on the scheduler ends after a
/// worker dies, before any shutdown: `run_until_exit`, an exit watch, a
/// completion watch, the exit-event subscription and `await_all_parked`,
/// each held on its own thread for a process parked on a LIVE worker, and
/// a `SenderHandle::call` made after the latch.
#[test]
fn every_wait_held_on_the_scheduler_ends_after_a_worker_dies() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let noop = return_module(&registry, atoms.intern("b184_waits_noop"));
    let parked = receive_then_return_module(&registry, atoms.intern("b184_waits_parked"));
    let scheduler = scheduler_with_workers(&registry, WORKERS);
    let subscription = scheduler
        .subscribe_exit_events()
        .unwrap_or_else(|| panic!("the one subscription"));
    let actor = spawn_actor(&scheduler, || Echo)
        .unwrap_or_else(|error| panic!("spawn the echo actor: {error}"));
    assert_eq!(
        actor.sender.call(7),
        Ok(7),
        "the actor answers while all run"
    );
    let pid = spawn_parked_on(&scheduler, &noop, &parked, LIVE);
    // A gate holds one worker in a slice, so not every worker is parked and
    // `await_all_parked` waits until the failure ends it.
    let (started, starts) = crossbeam_channel::unbounded();
    let (release_gate, gate) = crossbeam_channel::unbounded();
    let gate_pid = scheduler
        .spawn_native(held_slice(&started, &gate, false))
        .unwrap_or_else(|error| panic!("spawn the gate: {error}"));
    assert_eq!(starts.recv(), Ok(gate_pid));
    let ExitWatchState::Live(exit_watch) = scheduler.watch_exit(pid) else {
        panic!("pid {pid} is live");
    };
    let completion = completion_watch(&scheduler, pid);

    let run = {
        let held = Arc::clone(&scheduler);
        std::thread::spawn(move || held.run_until_exit(pid).map(|(reason, _)| reason))
    };
    let exit = std::thread::spawn(move || exit_watch.recv());
    let removal = std::thread::spawn(move || completion.recv());
    let event = std::thread::spawn(move || {
        loop {
            match subscription.recv() {
                Ok(ExitEvent::Exited {
                    pid: exited,
                    reason,
                }) if exited == pid => {
                    break Ok(reason);
                }
                Ok(_) => {}
                Err(error) => break Err(error),
            }
        }
    });
    let all_parked = {
        let held = Arc::clone(&scheduler);
        std::thread::spawn(move || held.await_all_parked())
    };

    assert!(scheduler.panic_worker_at_next_park(DEAD));
    let joined = |name: &str, result: std::thread::Result<String>| {
        result.unwrap_or_else(|payload| panic!("{name}: {}", panic_text(payload.as_ref())))
    };
    let run = joined("run_until_exit", run.join().map(|r| format!("{r:?}")));
    let exit = joined("exit watch", exit.join().map(|r| format!("{r:?}")));
    let removal = joined("completion watch", removal.join().map(|r| format!("{r:?}")));
    let event = joined("exit events", event.join().map(|r| format!("{r:?}")));
    let all_parked = joined(
        "await_all_parked",
        all_parked.join().map(|()| "returned".into()),
    );
    let call = actor.sender.call(8);
    release_gate
        .send(())
        .unwrap_or_else(|error| panic!("release the gate: {error}"));
    let shutdown = shutdown_on_another_thread(&scheduler);

    assert_eq!(
        run,
        format!("{:?}", Err::<ExitReason, _>(park_failure(DEAD)))
    );
    assert_eq!(
        exit,
        format!("{:?}", Ok::<_, ()>((pid, ExitReason::Killed)))
    );
    assert_eq!(
        removal,
        format!("{:?}", Ok::<_, ()>((pid, ExitReason::Killed)))
    );
    assert_eq!(event, format!("{:?}", Ok::<_, ()>(ExitReason::Killed)));
    assert_eq!(all_parked, "returned");
    assert_eq!(
        call,
        Err(ActorError::Spawn),
        "a call after the latch is refused"
    );
    assert_eq!(shutdown, Ok(()));
}

/// An actor that answers a call with its request.
struct Echo;

impl Actor for Echo {
    type Call = i64;
    type Reply = i64;
    type Cast = i64;

    fn handle_call(&mut self, request: i64, _ctx: &mut ActorContext<'_, '_>) -> i64 {
        request
    }

    fn handle_cast(&mut self, _request: i64, _ctx: &mut ActorContext<'_, '_>) {}
}
