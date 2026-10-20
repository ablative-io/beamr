//! B-181: tickless idle. (Base-run copy: the idle-count red, which needs
//! R2's idle count, is held out; it is a compile red at the base.)
//! Every wake source is ordered against the park (R2),
//! and an idle worker parks with no timeout, or until the earliest armed
//! timer, while the polled sources wake it (R3).
//! EMBEDDER-COMPOSITION-SPEC §3.8, §3.8.1 (rows W1-W24) and §3.8.2 (C1-C4).
//!
//! Every wait in this file ends on an event: a process exit (an `ExitWatch`),
//! a reply on a channel, or a park hook firing. None ends on a clock.
//!
//! R2's reds fail on a named event, never on the harness's clock: the
//! `left_park` hook reports each time a worker leaves its park, whether the
//! park ended on its timeout with no wake edge, and what was pending for
//! that worker (pids in `woken`, injected spawns, stealable work). At the
//! base, where a lost wake was recovered only by the 5 ms timeout, every
//! loss shows as such a report, and the test fails naming it
//! ("worker 0 left its park on the clock with no wake edge while pids [17]
//! in woken was pending"). After R3 no normal worker's park has a timeout.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::atom::{Atom, AtomTable};
use crate::loader::Instruction;
use crate::loader::decode::compact::Operand;
use crate::module::{Module, ModuleRegistry};
use crate::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use crate::process::ExitReason;
use crate::term::Term;
use crate::timer::{TimerKind, TimerWheel};

use super::execution::{self, ObservedParkTimeout};
use super::tests::{deliver_atom, receive_then_return_module, test_module};
use super::{ExitWatchState, NativeBifs, ParkGap, Scheduler, SchedulerConfig, lock_or_recover};

/// The stress size B-181 R2 names.
const STRESS_ROUNDS: usize = 10_000;

fn scheduler_with_workers(registry: &Arc<ModuleRegistry>, workers: usize) -> Scheduler {
    Scheduler::new(
        SchedulerConfig {
            thread_count: Some(workers),
            ..SchedulerConfig::default()
        },
        Arc::clone(registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"))
}

/// Block until `pid` exits, on the exit watch: an event, never a clock.
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

fn exit_value(scheduler: &Scheduler, pid: u64) -> Option<Term> {
    scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root())
}

fn return_module(registry: &ModuleRegistry, name: Atom) -> Arc<Module> {
    registry.insert(test_module(name, vec![Instruction::Return]))
}

/// `receive _ -> ok after Millis -> true end`: the timeout path returns TRUE.
fn receive_after_ms_module(registry: &ModuleRegistry, name: Atom, millis: u64) -> Arc<Module> {
    registry.insert(test_module(
        name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::LoopRec {
                fail: Operand::Label(2),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label { label: 2 },
            Instruction::WaitTimeout {
                fail: Operand::Label(1),
                timeout: Operand::Unsigned(millis),
            },
            Instruction::Timeout,
            Instruction::Move {
                source: Operand::Atom(Some(Atom::TRUE)),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
    ))
}

/// Spawn a no-op process when the next bytecode spawn would land on any
/// worker other than `worker`, so the following spawn lands on `worker`
/// (bytecode spawns are placed round-robin by `spawn_counter`).
fn steer_next_spawn_to(scheduler: &Scheduler, module: &Arc<Module>, worker: usize) {
    let workers = scheduler.shared.thread_count;
    while scheduler.shared.spawn_counter.load(Ordering::Acquire) % workers != worker {
        let pid = scheduler
            .spawn_process(module)
            .unwrap_or_else(|error| panic!("steering spawn: {error}"));
        assert_eq!(await_exit(scheduler, pid), ExitReason::Normal);
    }
}

/// B-181 R2's loss detector: every time a worker leaves its park on the
/// clock, with no wake edge, while work for it is pending, the report is
/// kept, named; the test asserts none after each wake it makes.
struct LossDetector {
    lost: Arc<Mutex<Vec<String>>>,
}

fn install_loss_detector(scheduler: &Scheduler) -> LossDetector {
    let lost = Arc::new(Mutex::new(Vec::new()));
    let hook_lost = Arc::clone(&lost);
    let installed = scheduler
        .shared
        .park_hooks
        .left_park
        .set(Box::new(move |left| {
            if let Some(line) = left.lost_wake() {
                lock_or_recover(&hook_lost).push(line);
            }
        }));
    assert!(installed.is_ok(), "left-park hook installed once");
    LossDetector { lost }
}

impl LossDetector {
    fn assert_no_lost_wake(&self) {
        if let Some(first) = lock_or_recover(&self.lost).first() {
            panic!("{first}");
        }
    }
}

/// The worker the parked `pid` is registered under in the wait set.
fn registered_worker(scheduler: &Scheduler, pid: u64) -> Option<usize> {
    lock_or_recover(&scheduler.shared.wait_set)
        .waiting
        .get(&pid)
        .copied()
}

/// Install a waiting hook that reports every `(worker, timeout)` a worker
/// starts to wait with. The hook runs with `wait_set` held; it only sends.
fn waiting_events(scheduler: &Scheduler) -> mpsc::Receiver<(usize, Option<Duration>)> {
    let (sender, receiver) = mpsc::channel();
    let sender = Mutex::new(sender);
    let installed = scheduler
        .shared
        .park_hooks
        .waiting
        .set(Box::new(move |worker, timeout| {
            // A dropped receiver means the test is done reading; the send
            // result carries nothing the park needs.
            let _receiver_gone = lock_or_recover(&sender).send((worker, timeout));
        }));
    assert!(installed.is_ok(), "waiting hook installed once");
    receiver
}

/// Wait for the next time `worker` starts to wait with a deadline.
fn next_bounded_wait(
    events: &mpsc::Receiver<(usize, Option<Duration>)>,
    worker: usize,
) -> Duration {
    loop {
        let (index, timeout) = events
            .recv()
            .unwrap_or_else(|error| panic!("waiting events: {error}"));
        if index == worker
            && let Some(timeout) = timeout
        {
            return timeout;
        }
    }
}

/// An idle-gap handshake: the hook, once armed, tells the test the worker is
/// in its gap, then holds the worker there until the test says its wake is
/// done. The wake therefore lands after the worker's checks and before its
/// park, every time.
struct GapHandshake {
    armed: Arc<AtomicBool>,
    in_gap: mpsc::Receiver<usize>,
    release: mpsc::Sender<()>,
}

fn install_gap_handshake(scheduler: &Scheduler, worker: usize) -> GapHandshake {
    let armed = Arc::new(AtomicBool::new(false));
    let (in_gap_sender, in_gap) = mpsc::channel();
    let (release, release_receiver) = mpsc::channel::<()>();
    let hook_armed = Arc::clone(&armed);
    let in_gap_sender = Mutex::new(in_gap_sender);
    let release_receiver = Mutex::new(release_receiver);
    let installed = scheduler
        .shared
        .park_hooks
        .idle_gap
        .set(Box::new(move |_shared, index| {
            if index != worker || !hook_armed.swap(false, Ordering::AcqRel) {
                return;
            }
            lock_or_recover(&in_gap_sender)
                .send(index)
                .unwrap_or_else(|error| panic!("publish idle gap: {error}"));
            // A disconnected sender also releases the worker if the test fails.
            let _released_or_gone = lock_or_recover(&release_receiver).recv();
        }));
    assert!(installed.is_ok(), "idle-gap hook installed once");
    GapHandshake {
        armed,
        in_gap,
        release,
    }
}

impl GapHandshake {
    fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }

    fn wait_in_gap(&self) -> usize {
        self.in_gap
            .recv()
            .unwrap_or_else(|error| panic!("worker reaches its idle gap: {error}"))
    }

    fn release(&self) {
        self.release
            .send(())
            .unwrap_or_else(|error| panic!("release the worker: {error}"));
    }
}

/// Spawn `count` receive-then-return processes and wait until every one is
/// registered in the wait set (the park-gap hook reports each registration).
fn spawn_parked_receivers(scheduler: &Scheduler, module: &Arc<Module>, count: usize) -> Vec<u64> {
    let (sender, registered) = mpsc::channel();
    let sender = Mutex::new(sender);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap == ParkGap::WaitRegistered && pid != shared.standard_io_pid {
            lock_or_recover(&sender)
                .send(pid)
                .unwrap_or_else(|error| panic!("publish registration: {error}"));
        }
    }));
    let pids: Vec<u64> = (0..count)
        .map(|_| {
            scheduler
                .spawn_process(module)
                .unwrap_or_else(|error| panic!("spawn receiver: {error}"))
        })
        .collect();
    let mut seen = BTreeSet::new();
    while seen.len() < count {
        seen.insert(
            registered
                .recv()
                .unwrap_or_else(|error| panic!("registration events: {error}")),
        );
    }
    *lock_or_recover(&scheduler.shared.park_gap_hook) = None;
    pids
}

// ---------------------------------------------------------------------------
// R2: no wake is lost between a worker's checks and its park.
// ---------------------------------------------------------------------------

/// R2 red 1, the stress: 10,000 `wake_process` calls from the test thread,
/// each racing the worker that just ran the previous process and found its
/// queue empty. Wakes are sent in worker order, so consecutive wakes target
/// the worker that just went idle. Ends on each process's exit.
#[test]
fn ten_thousand_wakes_racing_idle_workers_are_never_lost() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let module = receive_then_return_module(&registry, atoms.intern("tickless_wake_stress"));
    let scheduler = scheduler_with_workers(&registry, 2);
    let pids = spawn_parked_receivers(&scheduler, &module, STRESS_ROUNDS);
    let mut ordered: Vec<(usize, u64)> = pids
        .iter()
        .map(|pid| {
            let worker = registered_worker(&scheduler, *pid)
                .unwrap_or_else(|| panic!("receiver {pid} is parked"));
            (worker, *pid)
        })
        .collect();
    ordered.sort_unstable();
    let detector = install_loss_detector(&scheduler);
    for (_, pid) in &ordered {
        deliver_atom(&scheduler.shared, *pid, Atom::OK);
        execution::wake_process(&scheduler.shared, *pid);
        assert_eq!(await_exit(&scheduler, *pid), ExitReason::Normal);
        detector.assert_no_lost_wake();
    }
    for (_, pid) in &ordered {
        assert_eq!(exit_value(&scheduler, *pid), Some(Term::atom(Atom::OK)));
    }
    scheduler.shutdown();
}

/// R2 red 1, the exact interleaving: one wake from another thread lands in
/// the worker's idle gap, after its checks and before its park.
#[test]
fn a_wake_in_the_idle_gap_returns_the_worker_to_its_loop() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let receiver = receive_then_return_module(&registry, atoms.intern("tickless_gap_wake"));
    let noop = return_module(&registry, atoms.intern("tickless_gap_wake_noop"));
    let scheduler = scheduler_with_workers(&registry, 1);
    let pids = spawn_parked_receivers(&scheduler, &receiver, 1);
    let target = pids[0];
    let detector = install_loss_detector(&scheduler);
    let gap = install_gap_handshake(&scheduler, 0);
    gap.arm();
    // Any work passes the worker through its gap; a no-op spawn is enough.
    let noop_pid = scheduler
        .spawn_process(&noop)
        .unwrap_or_else(|error| panic!("spawn no-op: {error}"));
    assert_eq!(gap.wait_in_gap(), 0);
    deliver_atom(&scheduler.shared, target, Atom::OK);
    execution::wake_process(&scheduler.shared, target);
    gap.release();
    assert_eq!(await_exit(&scheduler, noop_pid), ExitReason::Normal);
    assert_eq!(await_exit(&scheduler, target), ExitReason::Normal);
    detector.assert_no_lost_wake();
    assert_eq!(exit_value(&scheduler, target), Some(Term::atom(Atom::OK)));
    scheduler.shutdown();
}

/// R2 red 2, the stress for injected spawns: 10,000 spawns, each injected
/// while the one worker has just finished the previous process.
#[test]
fn ten_thousand_injected_spawns_racing_an_idle_worker_all_run() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let module = return_module(&registry, atoms.intern("tickless_spawn_stress"));
    let scheduler = scheduler_with_workers(&registry, 1);
    let detector = install_loss_detector(&scheduler);
    for _ in 0..STRESS_ROUNDS {
        let pid = scheduler
            .spawn_process(&module)
            .unwrap_or_else(|error| panic!("spawn: {error}"));
        assert_eq!(await_exit(&scheduler, pid), ExitReason::Normal);
        detector.assert_no_lost_wake();
    }
    scheduler.shutdown();
}

/// R2 red 2, the exact interleaving for an injected spawn.
#[test]
fn a_spawn_injected_in_the_idle_gap_runs() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let module = return_module(&registry, atoms.intern("tickless_gap_spawn"));
    let scheduler = scheduler_with_workers(&registry, 1);
    let detector = install_loss_detector(&scheduler);
    let gap = install_gap_handshake(&scheduler, 0);
    gap.arm();
    let first = scheduler
        .spawn_process(&module)
        .unwrap_or_else(|error| panic!("spawn first: {error}"));
    assert_eq!(gap.wait_in_gap(), 0);
    let second = scheduler
        .spawn_process(&module)
        .unwrap_or_else(|error| panic!("spawn second: {error}"));
    gap.release();
    assert_eq!(await_exit(&scheduler, first), ExitReason::Normal);
    assert_eq!(await_exit(&scheduler, second), ExitReason::Normal);
    detector.assert_no_lost_wake();
    scheduler.shutdown();
}

/// R2 red 2, a timer fired by worker 0 while every other worker parks: the
/// fire lands in worker 1's idle gap. Worker 0 ticks the wheel; worker 1
/// parks unbounded. The hook holds worker 1 in its gap until worker 0's fire
/// has moved the process to worker 1's woken list, waiting on that fire's
/// own notify.
#[test]
fn a_timer_fired_by_worker_zero_into_an_idle_gap_runs_its_process() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let module = receive_after_ms_module(&registry, atoms.intern("tickless_gap_timer"), 50);
    let noop = return_module(&registry, atoms.intern("tickless_gap_timer_noop"));
    let scheduler = scheduler_with_workers(&registry, 2);
    steer_next_spawn_to(&scheduler, &noop, 1);
    let detector = install_loss_detector(&scheduler);
    let armed = Arc::new(AtomicBool::new(true));
    let hook_armed = Arc::clone(&armed);
    let installed = scheduler
        .shared
        .park_hooks
        .idle_gap
        .set(Box::new(move |shared, index| {
            if index != 1 || !hook_armed.load(Ordering::Acquire) {
                return;
            }
            let mut wait_set = lock_or_recover(&shared.wait_set);
            if !wait_set.waiting.values().any(|worker| *worker == 1) {
                // Not yet parked behind its timer: this gap is not the one.
                return;
            }
            hook_armed.store(false, Ordering::Release);
            while !wait_set.woken.iter().any(|(_, worker)| *worker == 1) {
                wait_set = match shared.wake_condvar.wait(wait_set) {
                    Ok(guard) => guard,
                    Err(error) => error.into_inner(),
                };
            }
        }));
    assert!(installed.is_ok(), "idle-gap hook installed once");
    let pid = scheduler
        .spawn_process(&module)
        .unwrap_or_else(|error| panic!("spawn timer process: {error}"));
    assert_eq!(await_exit(&scheduler, pid), ExitReason::Normal);
    detector.assert_no_lost_wake();
    assert_eq!(exit_value(&scheduler, pid), Some(Term::atom(Atom::TRUE)));
    assert!(!armed.load(Ordering::Acquire), "the gap was exercised");
    scheduler.shutdown();
}

/// A native that reports where it started, then blocks until the test
/// releases it: the "hook the test holds" of C1's red.
struct HeldByTest {
    started: Option<crossbeam_channel::Sender<(u64, String)>>,
    release: crossbeam_channel::Receiver<()>,
}

impl NativeHandler for HeldByTest {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        if let Some(started) = self.started.take() {
            let thread = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string();
            started
                .send((context.self_pid(), thread))
                .unwrap_or_else(|error| panic!("publish start: {error}"));
        }
        // A disconnected sender also releases the worker if the test fails.
        let _released_or_gone = self.release.recv();
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

fn held_factory(
    started: &crossbeam_channel::Sender<(u64, String)>,
    release: &crossbeam_channel::Receiver<()>,
) -> crate::native::native_process::NativeHandlerFactory {
    let started = started.clone();
    let release = release.clone();
    Box::new(move || {
        Box::new(HeldByTest {
            started: Some(started.clone()),
            release: release.clone(),
        })
    })
}

/// C1's red: two workers; the owner blocked inside a slice on a hook the
/// test holds, a second process pushed behind it on the owner's queue. The
/// second process runs on the other worker while the owner is still blocked.
#[test]
fn surplus_behind_a_blocked_owner_runs_on_the_idle_worker() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = scheduler_with_workers(&registry, 2);
    let detector = install_loss_detector(&scheduler);
    let (started, starts) = crossbeam_channel::unbounded();
    let (release_gate, gate) = crossbeam_channel::unbounded::<()>();
    let (release_pair, pair) = crossbeam_channel::unbounded::<()>();
    // Natives are placed on worker 0. The gate holds worker 0 while the pair
    // is spawned, so both land in its woken list and one drain pushes both.
    let gate_pid = scheduler
        .spawn_native(held_factory(&started, &gate))
        .unwrap_or_else(|error| panic!("spawn gate: {error}"));
    let (pid, gate_thread) = starts
        .recv()
        .unwrap_or_else(|error| panic!("gate starts: {error}"));
    assert_eq!(pid, gate_pid);
    let first = scheduler
        .spawn_native(held_factory(&started, &pair))
        .unwrap_or_else(|error| panic!("spawn first: {error}"));
    let second = scheduler
        .spawn_native(held_factory(&started, &pair))
        .unwrap_or_else(|error| panic!("spawn second: {error}"));
    release_gate
        .send(())
        .unwrap_or_else(|error| panic!("release gate: {error}"));
    assert_eq!(await_exit(&scheduler, gate_pid), ExitReason::Normal);
    // Both of the pair start while neither is released: the one the owner
    // runs holds the owner, so the other ran on the idle worker.
    let (pid_a, thread_a) = starts
        .recv()
        .unwrap_or_else(|error| panic!("first of the pair starts: {error}"));
    let (pid_b, thread_b) = starts
        .recv()
        .unwrap_or_else(|error| panic!("second of the pair starts: {error}"));
    detector.assert_no_lost_wake();
    assert_eq!(
        BTreeSet::from([pid_a, pid_b]),
        BTreeSet::from([first, second])
    );
    assert_ne!(thread_a, thread_b, "the pair ran on two workers");
    assert!(
        thread_a == gate_thread || thread_b == gate_thread,
        "one of the pair ran on the owner"
    );
    for _ in 0..2 {
        release_pair
            .send(())
            .unwrap_or_else(|error| panic!("release pair: {error}"));
    }
    assert_eq!(await_exit(&scheduler, first), ExitReason::Normal);
    assert_eq!(await_exit(&scheduler, second), ExitReason::Normal);
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// R3: an idle worker parks with no timeout, or until the earliest armed
// timer, and the polled sources wake it.
// ---------------------------------------------------------------------------

/// R3 red: an idle scheduler with no timer armed and no IO in flight parks
/// every worker with no timeout, read at the existing seam.
#[test]
fn an_idle_scheduler_parks_every_worker_with_no_timeout() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let module = return_module(&registry, atoms.intern("tickless_idle_unbounded"));
    let scheduler = Scheduler::new(
        SchedulerConfig::default(),
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let workers = scheduler.shared.thread_count;
    let events = waiting_events(&scheduler);
    let pid = scheduler
        .spawn_process(&module)
        .unwrap_or_else(|error| panic!("spawn: {error}"));
    assert_eq!(await_exit(&scheduler, pid), ExitReason::Normal);
    let mut reported = BTreeSet::new();
    while reported.len() < workers {
        let (worker, _timeout) = events
            .recv()
            .unwrap_or_else(|error| panic!("waiting events: {error}"));
        reported.insert(worker);
    }
    for worker in 0..workers {
        assert_eq!(
            scheduler.worker_observed_park_timeout(worker),
            ObservedParkTimeout::Unbounded,
            "worker {worker} parks with no timeout"
        );
    }
    assert_eq!(scheduler.observed_park_timeout_millis(), None);
    scheduler.shutdown();
}

/// R3 red: one timer armed at a deadline D by a process on worker 1. Worker
/// 0 parks until D, the receive-after fires, and between the arming and the
/// timeout reply exactly one park timed out on worker 0 and none on worker 1.
#[test]
fn one_armed_timer_costs_worker_zero_one_timed_out_park() {
    const DEADLINE: Duration = Duration::from_millis(300);
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let module = receive_after_ms_module(
        &registry,
        atoms.intern("tickless_one_timer"),
        u64::try_from(DEADLINE.as_millis()).unwrap_or(u64::MAX),
    );
    let noop = return_module(&registry, atoms.intern("tickless_one_timer_noop"));
    let scheduler = scheduler_with_workers(&registry, 2);
    steer_next_spawn_to(&scheduler, &noop, 1);
    let events = waiting_events(&scheduler);
    let before = [
        scheduler.worker_park_counts(0),
        scheduler.worker_park_counts(1),
    ];
    let spawned_at = Instant::now();
    let pid = scheduler
        .spawn_process(&module)
        .unwrap_or_else(|error| panic!("spawn timer process: {error}"));
    let worker_zero_wait = next_bounded_wait(&events, 0);
    assert!(
        worker_zero_wait > DEADLINE / 2 && worker_zero_wait <= DEADLINE,
        "worker 0 parks until D, not on a tick: {worker_zero_wait:?}"
    );
    assert_eq!(await_exit(&scheduler, pid), ExitReason::Normal);
    let elapsed = spawned_at.elapsed();
    assert_eq!(exit_value(&scheduler, pid), Some(Term::atom(Atom::TRUE)));
    assert!(elapsed >= DEADLINE, "fired at its deadline, not early");
    let after = [
        scheduler.worker_park_counts(0),
        scheduler.worker_park_counts(1),
    ];
    assert_eq!(
        after[0].timed_out - before[0].timed_out,
        1,
        "worker 0 timed out once, at D"
    );
    assert_eq!(
        after[1].timed_out - before[1].timed_out,
        0,
        "worker 1 never timed out"
    );
    scheduler.shutdown();
}

/// A native that parks, then reports the slice it was woken into.
struct ReportsWake {
    woken: crossbeam_channel::Sender<u64>,
    slices: usize,
}

impl NativeHandler for ReportsWake {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        self.slices += 1;
        if self.slices == 1 {
            return NativeOutcome::Wait;
        }
        self.woken
            .send(context.self_pid())
            .unwrap_or_else(|error| panic!("publish wake: {error}"));
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

/// R3 red: a file completion published while worker 0 is parked wakes it by
/// an edge, not by its clock. The test submits a read on the scheduler's own
/// file ring and registers it for a parked process the way the file facility
/// does. The `waiting` hook completes the read from inside worker 0's next
/// wait-start, so worker 0 is parked, every time, when the ring publishes;
/// the `left_park` hook reports how worker 0 left that park. At the base
/// the ring had no notifier and worker 0 found the completion only on its
/// 5 ms timeout (W13, W14).
#[test]
fn a_file_completion_wakes_a_process_with_every_worker_parked() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = scheduler_with_workers(&registry, 2);
    let (registered_sender, registered) = mpsc::channel();
    let registered_sender = Mutex::new(registered_sender);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap == ParkGap::WaitRegistered && pid != shared.standard_io_pid {
            lock_or_recover(&registered_sender)
                .send(pid)
                .unwrap_or_else(|error| panic!("publish registration: {error}"));
        }
    }));
    let (woken_sender, woken) = crossbeam_channel::unbounded();
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ReportsWake {
                woken: woken_sender.clone(),
                slices: 0,
            })
        }))
        .unwrap_or_else(|error| panic!("spawn file waiter: {error}"));
    assert_eq!(
        registered
            .recv()
            .unwrap_or_else(|error| panic!("waiter registers: {error}")),
        pid
    );
    *lock_or_recover(&scheduler.shared.park_gap_hook) = None;
    // The read blocks in the ring's worker until the test side writes, so
    // the completion is published only when the waiting hook writes.
    let (held, ring_side) = std::os::unix::net::UnixStream::pair()
        .unwrap_or_else(|error| panic!("socket pair: {error}"));
    let ring = scheduler
        .shared
        .file_io_ring
        .service()
        .unwrap_or_else(|| panic!("the default scheduler owns a file ring"));
    let op_id = ring.ring().submit(crate::io::IoOp::Read {
        fd: std::os::fd::AsRawFd::as_raw_fd(&ring_side),
        buf_len: 1,
        offset: u64::MAX,
    });
    scheduler
        .shared
        .file_io_pending
        .insert(op_id, (pid, crate::native::FileIoContinuation::Open));
    // Worker 0 leaves its park after the write: how, reported once.
    let written = Arc::new(AtomicBool::new(false));
    let (exits_sender, exits) = mpsc::channel();
    let exits_sender = Mutex::new(exits_sender);
    let exit_written = Arc::clone(&written);
    let installed = scheduler
        .shared
        .park_hooks
        .left_park
        .set(Box::new(move |left| {
            if left.worker == 0 && exit_written.swap(false, Ordering::AcqRel) {
                lock_or_recover(&exits_sender)
                    .send(left.by_clock_without_edge)
                    .unwrap_or_else(|error| panic!("publish worker 0's exit: {error}"));
            }
        }));
    assert!(installed.is_ok(), "left-park hook installed once");
    // The write happens in worker 0's wait-start, with wait_set held and
    // before the wait: worker 0 is parked when the ring publishes.
    let held = Mutex::new(Some(held));
    let hook_written = Arc::clone(&written);
    let nudged = Arc::new(AtomicBool::new(false));
    let hook_nudged = Arc::clone(&nudged);
    let installed = scheduler
        .shared
        .park_hooks
        .waiting
        .set(Box::new(move |worker, _timeout| {
            // Only the wait-start after the nudge ran: its notify is spent,
            // so nothing but the completion can end the wait that follows.
            if worker != 0 || !hook_nudged.load(Ordering::Acquire) {
                return;
            }
            if let Some(stream) = lock_or_recover(&held).take() {
                std::io::Write::write_all(&mut &stream, b"x")
                    .unwrap_or_else(|error| panic!("complete the read: {error}"));
                hook_written.store(true, Ordering::Release);
            }
        }));
    assert!(installed.is_ok(), "waiting hook installed once");
    // Pass worker 0 through one more wait-start: a native runs on worker 0
    // and stops, and worker 0 parks after it.
    let nudge = scheduler
        .spawn_native(Box::new(move || {
            Box::new(StopsAtOnce {
                ran: Arc::clone(&nudged),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn nudge: {error}"));
    assert_eq!(await_exit(&scheduler, nudge), ExitReason::Normal);
    let by_clock = exits
        .recv()
        .unwrap_or_else(|error| panic!("worker 0 leaves its park: {error}"));
    assert!(
        !by_clock,
        "worker 0 left its park on the clock with no wake edge after the file \
         completion was published: the completion has no wake edge"
    );
    assert_eq!(
        woken
            .recv()
            .unwrap_or_else(|error| panic!("completion wakes the waiter: {error}")),
        pid
    );
    assert_eq!(await_exit(&scheduler, pid), ExitReason::Normal);
    scheduler.shutdown();
}

/// A native that marks that it ran, then stops in its first slice.
struct StopsAtOnce {
    ran: Arc<AtomicBool>,
}

impl NativeHandler for StopsAtOnce {
    fn handle(&mut self, _context: &mut NativeContext<'_>) -> NativeOutcome {
        self.ran.store(true, Ordering::Release);
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

/// C4's red, on the wheel: a timer armed more than one lap out, the wheel
/// left untouched for longer than a lap before the deadline (a long park).
/// The catch-up walks at most one lap, fires nothing early, lands
/// `current_tick` on `target_tick`, and the timer fires once at its deadline.
#[test]
fn tick_at_catches_up_at_most_one_lap_and_fires_on_time() {
    const BUCKETS: usize = 8;
    let start = Instant::now();
    let mut wheel = TimerWheel::with_bucket_count(BUCKETS);
    let deadline = start + Duration::from_millis(30);
    let reference = wheel.schedule_at(
        start,
        Duration::from_millis(30),
        7,
        Term::atom(Atom::OK),
        TimerKind::Deliver,
    );
    assert!(wheel.tick_at(start + Duration::from_millis(1)).is_empty());
    let long_park_end = start + Duration::from_millis(29);
    let visits_before = wheel.bucket_visits();
    assert!(
        wheel.tick_at(long_park_end).is_empty(),
        "nothing fires before its deadline"
    );
    assert!(
        wheel.bucket_visits() - visits_before <= BUCKETS as u64,
        "the catch-up walks at most one lap: walked {}",
        wheel.bucket_visits() - visits_before
    );
    assert_eq!(wheel.current_tick(), wheel.target_tick(long_park_end));
    let fired = wheel.tick_at(deadline);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].reference, reference);
    assert_eq!(wheel.current_tick(), wheel.target_tick(deadline));
    assert!(
        wheel
            .tick_at(deadline + Duration::from_millis(40))
            .is_empty()
    );
    assert!(wheel.is_empty());
}

/// C4 through the scheduler: a receive-after more than one lap of the
/// scheduler's wheel out fires once, at its deadline. (A guard, not a red:
/// at the base the 5 ms tick keeps the walk short, so this passes there.)
#[test]
fn a_timer_more_than_a_lap_out_fires_once_at_its_deadline() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let lap = u64::try_from(TimerWheel::new().bucket_count()).unwrap_or(u64::MAX);
    let deadline = Duration::from_millis(lap + lap / 2);
    let module = receive_after_ms_module(
        &registry,
        atoms.intern("tickless_lap_timer"),
        u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
    );
    let scheduler = scheduler_with_workers(&registry, 2);
    let spawned_at = Instant::now();
    let pid = scheduler
        .spawn_process(&module)
        .unwrap_or_else(|error| panic!("spawn: {error}"));
    assert_eq!(await_exit(&scheduler, pid), ExitReason::Normal);
    assert!(spawned_at.elapsed() >= deadline, "never early");
    assert_eq!(exit_value(&scheduler, pid), Some(Term::atom(Atom::TRUE)));
    scheduler.shutdown();
}

/// R3 red (C2): with worker 0 parked until D, arming a timer after D runs no
/// edge; arming one before D wakes worker 0 once, it re-parks for the
/// earlier deadline, and that timer fires at its deadline. Both arms go
/// through the public `Scheduler::timers()`, from the test thread.
#[test]
fn arming_before_the_parked_deadline_wakes_worker_zero_and_after_it_does_not() {
    const D: Duration = Duration::from_millis(800);
    const EARLY: Duration = Duration::from_millis(100);
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let timed = receive_after_ms_module(
        &registry,
        atoms.intern("tickless_arm_d"),
        u64::try_from(D.as_millis()).unwrap_or(u64::MAX),
    );
    let receiver = receive_then_return_module(&registry, atoms.intern("tickless_arm_target"));
    let noop = return_module(&registry, atoms.intern("tickless_arm_noop"));
    let scheduler = scheduler_with_workers(&registry, 2);
    let targets = spawn_parked_receivers(&scheduler, &receiver, 1);
    let target = targets[0];
    steer_next_spawn_to(&scheduler, &noop, 1);
    let events = waiting_events(&scheduler);
    let timed_pid = scheduler
        .spawn_process(&timed)
        .unwrap_or_else(|error| panic!("spawn D process: {error}"));
    let parked_for = next_bounded_wait(&events, 0);
    assert!(parked_for > D / 2, "worker 0 parks until D: {parked_for:?}");
    let edges_before = lock_or_recover(scheduler.timers()).arm_wake_edges();
    {
        let mut wheel = lock_or_recover(scheduler.timers());
        let late = wheel.schedule(D * 3, target, Term::atom(Atom::OK), TimerKind::Deliver);
        assert_eq!(wheel.arm_wake_edges(), edges_before, "after D: no edge");
        assert!(wheel.cancel(late).is_some());
    }
    let armed_at = Instant::now();
    {
        let mut wheel = lock_or_recover(scheduler.timers());
        let _early = wheel.schedule(EARLY, target, Term::atom(Atom::OK), TimerKind::Deliver);
        assert_eq!(
            wheel.arm_wake_edges(),
            edges_before + 1,
            "before D: one edge"
        );
    }
    let reparked_for = next_bounded_wait(&events, 0);
    assert!(
        reparked_for <= EARLY,
        "worker 0 re-parks for the earlier deadline: {reparked_for:?}"
    );
    assert_eq!(await_exit(&scheduler, target), ExitReason::Normal);
    assert!(
        armed_at.elapsed() >= EARLY,
        "the earlier timer is never early"
    );
    assert_eq!(exit_value(&scheduler, target), Some(Term::atom(Atom::OK)));
    assert_eq!(await_exit(&scheduler, timed_pid), ExitReason::Normal);
    assert_eq!(
        exit_value(&scheduler, timed_pid),
        Some(Term::atom(Atom::TRUE))
    );
    scheduler.shutdown();
}
