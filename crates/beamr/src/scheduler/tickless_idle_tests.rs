//! B-181: tickless idle. Every wake source is ordered against the park (R2),
//! and an idle worker parks with no timeout, or until the earliest armed
//! timer, while the polled sources wake it (R3).
//! EMBEDDER-COMPOSITION-SPEC §3.8, §3.8.1 (rows W1-W24) and §3.8.2 (C1-C4).
//!
//! Every wait in these files ends on an event: a process exit (an `ExitWatch`),
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

mod conditions;
mod file_completion;
mod r2_races;
mod r3_parks;
