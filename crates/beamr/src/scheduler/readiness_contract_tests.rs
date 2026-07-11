//! Readiness-contract pinning suite (READINESS-CONTRACT-SPEC §2, clauses
//! C1–C4; enumerated in §2.5).
//!
//! This module is the normative pin for the shape-invariant readiness
//! contract every readiness consumer builds against. Each test drives a real
//! `Scheduler` and delivers every DURABLE MARKER exclusively through the
//! PUBLIC [`Scheduler::enqueue_atom_message`] surface; the scheduler
//! internals (`park_gap_hook`, `wait_set`, `process_bodies`) are touched ONLY
//! to inject a deterministic interleaving or to observe park/exit state —
//! never to deliver a marker by a private path. The C4 arm→probe test
//! additionally models a consumer-OWNED event source (a socket-buffer analog)
//! that is not a VM marker and is deliberately outside the VM's delivery
//! surface — see that test's deviation note.
//!
//! Determinism discipline (matching the existing park-gap suite in
//! `scheduler/tests.rs`): ORDERING is pinned with channel/latch handshakes,
//! never sleeps; LIVENESS is asserted with bounded polling loops; ABSENCE
//! (a park that must hold, a slice that must not run) is asserted over a
//! bounded observation window. The `park_gap_hook` runs synchronously on the
//! scheduler worker thread at the exact three-phase park gap, so a hook that
//! blocks holds the gap open while a helper thread lands a public delivery in
//! it — the same technique the existing lost-wakeup tests use, routed through
//! the public API.

use std::collections::HashMap as StdHashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::*;
use crate::atom::Atom;
use crate::loader::Instruction;
use crate::loader::decode::compact::Operand;
use crate::module::{Module, ModuleOrigin, ModuleRegistry, ResolvedImport, ResolvedImportTarget};
use crate::native::native_process::{
    NativeContext, NativeHandler, NativeHandlerFactory, NativeOutcome,
};
use crate::native::{Capability, NativeEntry, ProcessContext};
use crate::process::ExitReason;
use crate::term::Term;

// Distinct immediate atoms used as durable markers. Constants (not interned)
// so every scheduler sees the same identity without an atom-table round trip.
const MARKER: Atom = Atom::OK;
const DONE: Atom = Atom::TRUE;
const BACKSTOP: Atom = Atom::INFO;
const NEVER: Atom = Atom::BADKEY;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// A sticky one-way latch for deterministic ORDERING handshakes between the
/// test thread and a native handler (or gap hook) on a scheduler worker.
/// `Send + Sync` so it can be captured by a `NativeHandlerFactory` and by the
/// `Send + Sync` park-gap hook. Sticky: once raised, every `wait` returns at
/// once, so a handler that arms on each slice needs only one `raise`.
#[derive(Clone)]
struct Latch(Arc<(Mutex<bool>, Condvar)>);

impl Latch {
    fn new() -> Self {
        Latch(Arc::new((Mutex::new(false), Condvar::new())))
    }
    fn raise(&self) {
        let (lock, cvar) = &*self.0;
        *lock_or_recover(lock) = true;
        cvar.notify_all();
    }
    /// Deadline-bounded: a handshake that has not completed in 30s is a
    /// harness bug or a lost delivery — fail loudly instead of hanging the
    /// test process inside `Scheduler::drop`'s worker join.
    fn wait(&self) {
        let (lock, cvar) = &*self.0;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut raised = lock_or_recover(lock);
        while !*raised {
            let now = Instant::now();
            assert!(now < deadline, "latch handshake timed out");
            let (guard, _timeout) = cvar
                .wait_timeout(raised, deadline - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            raised = guard;
        }
    }
}

fn contract_scheduler() -> Arc<Scheduler> {
    let config = SchedulerConfig {
        thread_count: Some(1),
        dirty_cpu_threads: Some(1),
        dirty_io_threads: Some(1),
        dirty_queue_depth: Some(8),
        ..SchedulerConfig::default()
    };
    Arc::new(
        Scheduler::new(config, Arc::new(ModuleRegistry::new()))
            .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    )
}

fn wait_until(deadline_ms: u64, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_millis(deadline_ms);
    while !predicate() {
        assert!(Instant::now() <= deadline, "condition timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Assert `predicate` never holds within `window_ms` — the bounded negative
/// used for "the park must hold" / "no extra slice may run".
fn assert_absent(window_ms: u64, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_millis(window_ms);
    while Instant::now() < deadline {
        assert!(!predicate(), "observed a state the contract forbids");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Block until `pid` has completed its park and is registered in the wait set
/// (Present + in `waiting`), the precise "parked, not merely stored" state.
fn wait_parked(scheduler: &Scheduler, pid: u64) {
    wait_until(10_000, || {
        lock_or_recover(&scheduler.shared.wait_set)
            .waiting
            .contains_key(&pid)
    });
}

fn wait_exit(scheduler: &Scheduler, pid: u64) {
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
}

fn exit_value(scheduler: &Scheduler, pid: u64) -> Option<Term> {
    scheduler.shared.exit_results.get(&pid).map(|r| r.root())
}

fn observed(sink: &Arc<Mutex<Vec<Atom>>>) -> Vec<Atom> {
    lock_or_recover(sink).clone()
}

/// Install a one-shot park-gap hook that, at the FIRST `gap` for a
/// non-standard pid, holds the gap open while a helper thread runs `action`
/// through the PUBLIC scheduler API, then releases it. The hook only signals
/// and records the pid it fired for; it never calls a `Scheduler` method
/// itself (it holds `&SharedState`, not `&Scheduler`). Must be installed
/// BEFORE the target is spawned so the park cannot slip past it.
fn hold_gap_and<F>(scheduler: &Arc<Scheduler>, gap: ParkGap, action: F) -> JoinHandle<()>
where
    F: Fn(&Scheduler, u64) + Send + 'static,
{
    let go = Latch::new();
    let done = Latch::new();
    let pid_cell = Arc::new(AtomicU64::new(0));
    let fired = Arc::new(AtomicBool::new(false));
    {
        let go_h = go.clone();
        let done_h = done.clone();
        let cell_h = Arc::clone(&pid_cell);
        let fired_h = Arc::clone(&fired);
        *lock_or_recover(&scheduler.shared.park_gap_hook) =
            Some(Box::new(move |shared, g, pid| {
                if g != gap || pid == shared.standard_io_pid || fired_h.swap(true, Ordering::AcqRel)
                {
                    return;
                }
                cell_h.store(pid, Ordering::Release);
                go_h.raise();
                done_h.wait();
            }));
    }
    let sched = Arc::clone(scheduler);
    thread::spawn(move || {
        go.wait();
        let pid = pid_cell.load(Ordering::Acquire);
        action(&sched, pid);
        done.raise();
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Drains its mailbox each slice, records every atom it sees, and stops when
/// it has seen `stop_atom` (otherwise parks via `Wait`). `slices` counts every
/// invocation, so a wake that should schedule exactly one extra slice is
/// observable as an exact final count.
struct RecordingHandler {
    stop_atom: Atom,
    observed: Arc<Mutex<Vec<Atom>>>,
    slices: Arc<AtomicUsize>,
}

impl NativeHandler for RecordingHandler {
    fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
        self.slices.fetch_add(1, Ordering::AcqRel);
        let mut stop = false;
        while let Some(term) = ctx.recv() {
            if let Some(atom) = term.as_atom() {
                lock_or_recover(&self.observed).push(atom);
                if atom == self.stop_atom {
                    stop = true;
                }
            }
        }
        if stop {
            NativeOutcome::Stop(ExitReason::Normal)
        } else {
            NativeOutcome::Wait
        }
    }
}

fn recording_factory(
    stop_atom: Atom,
    observed: &Arc<Mutex<Vec<Atom>>>,
    slices: &Arc<AtomicUsize>,
) -> NativeHandlerFactory {
    let observed = Arc::clone(observed);
    let slices = Arc::clone(slices);
    Box::new(move || {
        Box::new(RecordingHandler {
            stop_atom,
            observed: Arc::clone(&observed),
            slices: Arc::clone(&slices),
        })
    })
}

/// Blocks mid-slice on its first invocation to prove the process is Executing
/// while a delivery lands, then returns `Wait` WITHOUT draining. Later slices
/// drain and stop on the marker. Pins the executing-position merge (C1).
struct ExecutingHandler {
    marker: Atom,
    observed: Arc<Mutex<Vec<Atom>>>,
    in_slice: Latch,
    release: Latch,
    first_done: bool,
}

impl NativeHandler for ExecutingHandler {
    fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
        if !self.first_done {
            self.first_done = true;
            self.in_slice.raise();
            self.release.wait();
            // Return without draining: the marker delivered while we were
            // Executing must be merged at store-back and re-observed, not
            // relied upon to have been drained this slice.
            return NativeOutcome::Wait;
        }
        let mut stop = false;
        while let Some(term) = ctx.recv() {
            if let Some(atom) = term.as_atom() {
                lock_or_recover(&self.observed).push(atom);
                if atom == self.marker {
                    stop = true;
                }
            }
        }
        if stop {
            NativeOutcome::Stop(ExitReason::Normal)
        } else {
            NativeOutcome::Wait
        }
    }
}

/// Consumer-shaped slice (C4): drain bounded work, arm interest, block on an
/// ack (the arm→final-probe window), then re-probe. Every drain reads BOTH
/// event sources a real consumer slice has — the VM mailbox (durable markers)
/// and, when present, a consumer-owned "socket" queue (the event source the
/// final probe actually protects; the VM does not track its readiness).
/// `found_by_probe` records whether the FINAL PROBE (not the opening drain)
/// was what observed the marker.
struct ConsumerHandler {
    socket: Option<Arc<Mutex<Vec<Atom>>>>,
    stop_atom: Atom,
    observed: Arc<Mutex<Vec<Atom>>>,
    slices: Arc<AtomicUsize>,
    armed: Latch,
    ack: Latch,
    found_by_probe: Arc<AtomicBool>,
}

impl ConsumerHandler {
    fn drain(&self, ctx: &mut NativeContext<'_>) -> Vec<Atom> {
        let mut got = Vec::new();
        while let Some(term) = ctx.recv() {
            if let Some(atom) = term.as_atom() {
                got.push(atom);
            }
        }
        if let Some(socket) = &self.socket {
            got.append(&mut lock_or_recover(socket));
        }
        got
    }
}

impl NativeHandler for ConsumerHandler {
    fn handle(&mut self, ctx: &mut NativeContext<'_>) -> NativeOutcome {
        self.slices.fetch_add(1, Ordering::AcqRel);
        // Step 1: drain bounded work to WouldBlock.
        let opening = self.drain(ctx);
        lock_or_recover(&self.observed).extend_from_slice(&opening);
        if opening.contains(&self.stop_atom) {
            return NativeOutcome::Stop(ExitReason::Normal);
        }
        // Steps 2–3: arm interest, then the final probe. The block models the
        // window between arming and probing; the test injects a delivery into
        // it deterministically.
        self.armed.raise();
        self.ack.wait();
        let probe = self.drain(ctx);
        lock_or_recover(&self.observed).extend_from_slice(&probe);
        if probe.contains(&MARKER) {
            self.found_by_probe.store(true, Ordering::Release);
        }
        if probe.contains(&self.stop_atom) {
            return NativeOutcome::Stop(ExitReason::Normal);
        }
        // Step 4: no observable work — park.
        NativeOutcome::Wait
    }
}

// ---------------------------------------------------------------------------
// C1 — durable markers survive every race order (§2.1)
// ---------------------------------------------------------------------------

/// Pins READINESS-CONTRACT-SPEC §2.1 C1, PARKED position: a marker delivered
/// to a fully parked process lands in the mailbox and the wake makes it
/// runnable, so the marker is observed and the process exits.
#[test]
fn c1_marker_to_a_parked_process_wakes_and_is_observed() {
    let scheduler = contract_scheduler();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let pid = scheduler
        .spawn_native(recording_factory(MARKER, &seen, &slices))
        .expect("spawn native");

    wait_parked(&scheduler, pid);
    assert!(
        scheduler.enqueue_atom_message(pid, MARKER),
        "delivery to a live parked process returns true"
    );
    wait_exit(&scheduler, pid);

    assert!(observed(&seen).contains(&MARKER), "marker observed");
    scheduler.shutdown();
}

/// Pins §2.1 C1, MID-PARK store→register gap: a delivery in the gap between the
/// Wait arm's store-back and its wait-set registration is observed before the
/// process sleeps (the post-registration recheck self-wakes) and schedules the
/// process exactly once. The marker is landed through the PUBLIC API by a
/// helper thread while the gap is held open.
#[test]
fn c1_delivery_in_the_store_to_register_gap_is_observed_before_sleep() {
    let scheduler = contract_scheduler();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let delivered = Arc::new(AtomicBool::new(false));

    let delivered_hook = Arc::clone(&delivered);
    let handle = hold_gap_and(&scheduler, ParkGap::WaitStored, move |s, pid| {
        delivered_hook.store(s.enqueue_atom_message(pid, MARKER), Ordering::Release);
    });

    // stop_atom = DONE so the marker is recorded but does NOT end the process,
    // making the wake's extra slice observable in the final count.
    let pid = scheduler
        .spawn_native(recording_factory(DONE, &seen, &slices))
        .expect("spawn native");

    wait_until(10_000, || observed(&seen).contains(&MARKER));
    assert!(
        delivered.load(Ordering::Acquire),
        "in-gap delivery returned true"
    );
    // The marker wake scheduled exactly one extra slice: spawn park + marker.
    wait_parked(&scheduler, pid);
    assert_eq!(slices.load(Ordering::Acquire), 2, "scheduled exactly once");

    assert!(scheduler.enqueue_atom_message(pid, DONE));
    wait_exit(&scheduler, pid);
    assert_eq!(
        slices.load(Ordering::Acquire),
        3,
        "no lost or double wakeup"
    );
    handle.join().expect("helper joins");
    scheduler.shutdown();
}

/// Pins §2.1 C1, MID-PARK register→recheck gap: a delivery here moves the pid
/// from `waiting` to `woken` AND the recheck sees the message, yet the process
/// is scheduled exactly once (the recheck's self-wake backs off because its
/// `waiting` removal finds nothing).
#[test]
fn c1_delivery_in_the_register_to_recheck_gap_schedules_exactly_once() {
    let scheduler = contract_scheduler();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let delivered = Arc::new(AtomicBool::new(false));

    let delivered_hook = Arc::clone(&delivered);
    let handle = hold_gap_and(&scheduler, ParkGap::WaitRegistered, move |s, pid| {
        delivered_hook.store(s.enqueue_atom_message(pid, MARKER), Ordering::Release);
    });

    let pid = scheduler
        .spawn_native(recording_factory(DONE, &seen, &slices))
        .expect("spawn native");

    wait_until(10_000, || observed(&seen).contains(&MARKER));
    assert!(
        delivered.load(Ordering::Acquire),
        "in-gap delivery returned true"
    );
    wait_parked(&scheduler, pid);
    assert_eq!(slices.load(Ordering::Acquire), 2, "scheduled exactly once");

    assert!(scheduler.enqueue_atom_message(pid, DONE));
    wait_exit(&scheduler, pid);
    assert_eq!(slices.load(Ordering::Acquire), 3, "single additional slice");
    handle.join().expect("helper joins");
    scheduler.shutdown();
}

/// Pins §2.1 C1, EXECUTING position: a delivery to a process that is mid-slice
/// goes through pending metadata, is merged into the mailbox at store-back,
/// and — because the handler suspends (returns `Wait`) meanwhile — the process
/// is NOT left parked forever: the recheck sees the merged marker and the next
/// slice observes it.
#[test]
fn c1_delivery_while_executing_merges_at_store_back_and_wakes() {
    let scheduler = contract_scheduler();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let in_slice = Latch::new();
    let release = Latch::new();

    let (seen_f, in_slice_f, release_f) = (Arc::clone(&seen), in_slice.clone(), release.clone());
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ExecutingHandler {
                marker: MARKER,
                observed: Arc::clone(&seen_f),
                in_slice: in_slice_f.clone(),
                release: release_f.clone(),
                first_done: false,
            })
        }))
        .expect("spawn native");

    // The handler is now provably Executing (blocked mid-slice).
    in_slice.wait();
    let delivered = scheduler.enqueue_atom_message(pid, MARKER);
    release.raise();
    // Asserted only after the worker is released: a false return must fail
    // the test, not hang shutdown's worker join behind a blocked latch.
    assert!(delivered, "delivery to an executing process returns true");

    wait_exit(&scheduler, pid);
    assert!(
        observed(&seen).contains(&MARKER),
        "executing-position marker observed after store-back merge + recheck"
    );
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// C2 — observed-or-runnable, gated-suspension scope limit (§2.2)
// ---------------------------------------------------------------------------

static C2_AWAIT_RUNS: AtomicUsize = AtomicUsize::new(0);
static C2_PARKED: AtomicBool = AtomicBool::new(false);

/// Gated host-await native: parks via `request_await_suspend` (wake_on_message
/// = false), so a plain marker must NOT wake it — only its own completion may.
fn c2_gated_await_native(_args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    C2_AWAIT_RUNS.fetch_add(1, Ordering::AcqRel);
    let _call_id = context.request_await_suspend(None);
    C2_PARKED.store(true, Ordering::Release);
    Ok(Term::NIL)
}

fn build_module(name: Atom, code: Vec<Instruction>) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: StdHashMap::new(),
        label_index,
        code,
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        function_table: Vec::new(),
        line_table: Vec::new(),
        line_info: Vec::new(),
    }
}

/// Pins §2.2 C2 scope limit: a process parked under a GATED suspension retains
/// an enqueued marker in its mailbox but STAYS parked (no slice, native not
/// re-executed) until its own completion event arrives; the resume then
/// observes BOTH the completion (it drove the wake) and the retained marker
/// (drained past the await and returned as the exit value).
#[test]
fn c2_gated_suspension_retains_marker_and_observes_at_completion() {
    C2_AWAIT_RUNS.store(0, Ordering::Release);
    C2_PARKED.store(false, Ordering::Release);

    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Arc::new(
        Scheduler::new(
            SchedulerConfig {
                thread_count: Some(1),
                dirty_cpu_threads: Some(1),
                dirty_io_threads: Some(1),
                dirty_queue_depth: Some(8),
                ..SchedulerConfig::default()
            },
            Arc::clone(&registry),
        )
        .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    );
    let name = scheduler.shared.atom_table.intern("c2_gated");
    // await native (gated suspend), then receive the retained marker into x1,
    // return it as the exit value.
    let mut module = build_module(
        name,
        vec![
            Instruction::CallExt {
                arity: Operand::Unsigned(0),
                import: Operand::Unsigned(0),
            },
            Instruction::Label { label: 1 },
            Instruction::LoopRec {
                fail: Operand::Label(2),
                destination: Operand::X(1),
            },
            Instruction::RemoveMessage,
            Instruction::Move {
                source: Operand::X(1),
                destination: Operand::X(0),
            },
            Instruction::Return,
            Instruction::Label { label: 2 },
            Instruction::Wait {
                fail: Operand::Label(1),
            },
        ],
    );
    module.resolved_imports.push(ResolvedImport {
        module: name,
        function: name,
        arity: 0,
        target: ResolvedImportTarget::Native(NativeEntry {
            function: c2_gated_await_native,
            dirty_kind: None,
            capability: Capability::Pure,
        }),
    });
    let module = registry.insert(module);
    let pid = scheduler.spawn_process(&module);

    // The native has requested the await AND the process has completed its
    // park (wait-set registration), not merely stored its slot. `trap_exit`
    // would be a false surrogate here — `process_trap_exit` answers for
    // Executing slots too (`scheduler/mod.rs`), so only wait-set membership
    // pins "fully parked under the gate" before delivery.
    wait_until(10_000, || C2_PARKED.load(Ordering::Acquire));
    wait_parked(&scheduler, pid);

    // Marker retained but the gated park must hold: no wake, no re-execution.
    assert!(
        scheduler.enqueue_atom_message(pid, MARKER),
        "marker delivered to the gated-parked process returns true"
    );
    assert_absent(60, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
            || C2_AWAIT_RUNS.load(Ordering::Acquire) != 1
    });

    // The completion event — and only it — resumes the process; the resumed
    // bytecode then drains the retained marker.
    assert!(
        scheduler.wake_with_result(pid, Term::small_int(42)),
        "completion resumes the gated await"
    );
    wait_exit(&scheduler, pid);
    assert_eq!(
        exit_value(&scheduler, pid),
        Some(Term::atom(MARKER)),
        "retained marker observed at completion"
    );
    assert_eq!(
        C2_AWAIT_RUNS.load(Ordering::Acquire),
        1,
        "the await native was not re-executed by the marker"
    );
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// C3 — dead-pid semantics (§2.3)
// ---------------------------------------------------------------------------

/// Pins §2.3 C3: `enqueue_atom_message` returns false iff no live process
/// exists — both for a pid that never existed and for one that ran to
/// Stop(Normal) and was reaped.
#[test]
fn c3_enqueue_to_a_dead_or_absent_pid_returns_false() {
    let scheduler = contract_scheduler();

    // Never existed.
    assert!(
        !scheduler.enqueue_atom_message(999_999, MARKER),
        "delivery to an absent pid returns false"
    );

    // Ran to Stop(Normal) then reaped.
    let seen = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let pid = scheduler
        .spawn_native(recording_factory(MARKER, &seen, &slices))
        .expect("spawn native");
    wait_parked(&scheduler, pid);
    assert!(scheduler.enqueue_atom_message(pid, MARKER));
    wait_exit(&scheduler, pid);
    wait_until(10_000, || {
        !scheduler.shared.process_bodies.contains_key(&pid)
    });
    assert!(
        !scheduler.enqueue_atom_message(pid, MARKER),
        "delivery to a reaped pid returns false"
    );
    scheduler.shutdown();
}

/// Pins §2.3 C3: a true-returning enqueue followed by the target's death
/// before its next slice drops the marker harmlessly — the marker is
/// PROVABLY never observed and the target PROVABLY gets no further slice.
///
/// Determinism: both the enqueue and the kill land inside the held
/// `WaitStored` park gap. At that point the slot is stored (Present) but the
/// pid is not yet wait-set-registered, and the sole worker is the thread
/// blocked in the hook — so the enqueue's wake no-ops, and `exit_signal`'s
/// Present arm terminates and cleans up the process synchronously before the
/// hook releases. Death cannot lose the race to the woken slice because the
/// woken slice cannot exist yet.
///
/// (Deliberately NOT asserted: wait-set state. A kill landing in this gap
/// leaves the parker to register a now-dead pid in `waiting` after cleanup
/// has already swept it — a real, benign residue of the same gap, reported
/// separately; pinning it here would pin the defect as contract.)
#[test]
fn c3_true_then_death_before_next_slice_drops_the_marker_harmlessly() {
    let scheduler = contract_scheduler();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let delivered = Arc::new(AtomicBool::new(false));

    let delivered_hook = Arc::clone(&delivered);
    let handle = hold_gap_and(&scheduler, ParkGap::WaitStored, move |s, pid| {
        delivered_hook.store(s.enqueue_atom_message(pid, MARKER), Ordering::Release);
        s.exit_signal(0, pid, ExitReason::Kill)
            .expect("kill delivered");
    });

    // stop_atom = NEVER: the process can only die via the external kill.
    let pid = scheduler
        .spawn_native(recording_factory(NEVER, &seen, &slices))
        .expect("spawn native");

    wait_exit(&scheduler, pid);
    handle.join().expect("helper joins");
    assert!(
        delivered.load(Ordering::Acquire),
        "enqueue to the live (stored, not yet parked) process returned true"
    );
    // The marker was dropped with the mailbox: never observed, and the target
    // never ran again — the spawn slice stays the only slice.
    assert!(
        observed(&seen).is_empty(),
        "marker dropped with the dead mailbox, never observed"
    );
    assert_absent(60, || slices.load(Ordering::Acquire) != 1);
    // Cleanup removed the process outright.
    assert!(
        !scheduler.shared.process_bodies.contains_key(&pid),
        "process body reaped"
    );

    // Health: the scheduler is not wedged — a new process runs to completion.
    let seen2 = Arc::new(Mutex::new(Vec::new()));
    let slices2 = Arc::new(AtomicUsize::new(0));
    let pid2 = scheduler
        .spawn_native(recording_factory(MARKER, &seen2, &slices2))
        .expect("spawn native");
    wait_parked(&scheduler, pid2);
    assert!(scheduler.enqueue_atom_message(pid2, MARKER));
    wait_exit(&scheduler, pid2);
    assert!(
        observed(&seen2).contains(&MARKER),
        "scheduler stayed healthy"
    );
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// C4 — consumer discipline: register-before-probe, probe-before-park (§2.4/2.5)
// ---------------------------------------------------------------------------

/// Pins §2.4/§2.5 C4, arm→probe window: a readiness event landing in the exact
/// window between arming interest and the final probe is caught BY THE PROBE,
/// in the arming slice, before any park — the consumer never relies on a wake
/// for it.
///
/// DEVIATION (reported): the brief specifies the probe as a mailbox re-drain
/// and the injected event as `enqueue_atom_message`. That cannot demonstrate
/// "seen by the probe" against beamr: a marker delivered mid-slice lands in
/// `pending_io_messages` and is merged into the mailbox only at store-back
/// (`execution/core.rs:394`), so it is INVISIBLE to a same-slice `ctx.recv()`
/// probe — mid-slice VM markers are instead protected by the C1
/// store-back+recheck (see test 9). The probe is therefore modeled over a
/// consumer-owned event source (the socket-syscall analog liminal's real
/// probe uses), which CAN change mid-slice — that is exactly the source class
/// the final probe exists to protect. The public delivery contract is still
/// causally exercised, not assumed: the in-window `enqueue_atom_message`
/// BACKSTOP must be true AND must be OBSERVED by the next slice's drain
/// (store-back merge → recheck self-wake), so this test fails if the
/// executing-slot public delivery path is broken.
#[test]
fn c4_delivery_between_arm_and_final_probe_is_seen_by_the_probe() {
    let scheduler = contract_scheduler();
    let socket = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let armed = Latch::new();
    let ack = Latch::new();
    let found_by_probe = Arc::new(AtomicBool::new(false));

    let (socket_f, seen_f, slices_f) =
        (Arc::clone(&socket), Arc::clone(&seen), Arc::clone(&slices));
    let (armed_f, ack_f, probe_f) = (armed.clone(), ack.clone(), Arc::clone(&found_by_probe));
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ConsumerHandler {
                socket: Some(Arc::clone(&socket_f)),
                stop_atom: DONE,
                observed: Arc::clone(&seen_f),
                slices: Arc::clone(&slices_f),
                armed: armed_f.clone(),
                ack: ack_f.clone(),
                found_by_probe: Arc::clone(&probe_f),
            })
        }))
        .expect("spawn native");

    // Interest is armed; we are in the arm→probe window. Land both the
    // consumer-owned event and the public backstop marker in it.
    armed.wait();
    lock_or_recover(&socket).push(MARKER);
    let backstop_delivered = scheduler.enqueue_atom_message(pid, BACKSTOP);
    ack.raise();
    // Asserted only after the worker is released: a false return must fail
    // the test, not hang shutdown's worker join behind a blocked latch.
    assert!(
        backstop_delivered,
        "the durable backstop marker delivers to the executing process"
    );

    // Slice 1's probe caught the socket event; the handler then parks, the
    // store-back merge + recheck wake slice 2, whose opening drain observes
    // the BACKSTOP — the causal pin on executing-slot public delivery.
    wait_until(10_000, || observed(&seen).contains(&BACKSTOP));
    assert!(
        found_by_probe.load(Ordering::Acquire),
        "the final probe observed the in-window delivery"
    );
    assert_eq!(
        observed(&seen),
        vec![MARKER, BACKSTOP],
        "probe caught the socket event in slice 1, before the mid-slice \
         backstop became visible in slice 2"
    );
    wait_parked(&scheduler, pid);
    assert_eq!(slices.load(Ordering::Acquire), 2, "scheduled exactly once");

    assert!(scheduler.enqueue_atom_message(pid, DONE));
    wait_exit(&scheduler, pid);
    assert_eq!(slices.load(Ordering::Acquire), 3, "single additional slice");
    scheduler.shutdown();
}

/// Pins §2.4 C4, after-probe race order: a delivery landing AFTER the final
/// probe (in the WaitStored park gap) is caught by the VM-side C1 recheck — the
/// backstop behind the consumer probe — and schedules the process exactly once.
/// The probe itself observed nothing (found_by_probe stays false); the marker
/// is caught by the next slice's drain. Together with test 8 this pins both
/// race orders of C4.
#[test]
fn c4_delivery_after_the_final_probe_is_caught_by_the_park_recheck() {
    let scheduler = contract_scheduler();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let armed = Latch::new();
    let ack = Latch::new();
    let found_by_probe = Arc::new(AtomicBool::new(false));
    let delivered = Arc::new(AtomicBool::new(false));

    // Deliver the marker in the WaitStored gap, AFTER the probe has parked.
    let delivered_hook = Arc::clone(&delivered);
    let handle = hold_gap_and(&scheduler, ParkGap::WaitStored, move |s, pid| {
        delivered_hook.store(s.enqueue_atom_message(pid, MARKER), Ordering::Release);
    });

    let (seen_f, slices_f) = (Arc::clone(&seen), Arc::clone(&slices));
    let (armed_f, ack_f, probe_f) = (armed.clone(), ack.clone(), Arc::clone(&found_by_probe));
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ConsumerHandler {
                socket: None,
                stop_atom: DONE,
                observed: Arc::clone(&seen_f),
                slices: Arc::clone(&slices_f),
                armed: armed_f.clone(),
                ack: ack_f.clone(),
                found_by_probe: Arc::clone(&probe_f),
            })
        }))
        .expect("spawn native");

    // Ack FIRST: the probe drains an empty mailbox and the handler parks; only
    // then does the gap hook land the marker.
    armed.wait();
    ack.raise();

    wait_until(10_000, || observed(&seen).contains(&MARKER));
    assert!(
        delivered.load(Ordering::Acquire),
        "in-gap delivery returned true"
    );
    assert!(
        !found_by_probe.load(Ordering::Acquire),
        "the marker was caught by the recheck, not the probe"
    );
    wait_parked(&scheduler, pid);
    assert_eq!(slices.load(Ordering::Acquire), 2, "scheduled exactly once");

    assert!(scheduler.enqueue_atom_message(pid, DONE));
    wait_exit(&scheduler, pid);
    assert_eq!(slices.load(Ordering::Acquire), 3, "single additional slice");
    handle.join().expect("helper joins");
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// Negative: bare wakes are not durable readiness signals (§2.4/§2.5)
// ---------------------------------------------------------------------------

/// Pins §2.4/§2.5: a bare `wake_process` issued while the target is in the
/// store→register gap (not yet registered in `waiting`) is LOST — the wake
/// no-ops and the process parks with no pending schedule and gets no further
/// slice. This is exactly WHY the contract forbids bare wakes as readiness
/// signals and demands durable markers. A subsequent real marker proves the
/// process was parked-alive, not dead.
#[test]
fn bare_wake_before_registration_is_lost_which_is_why_markers_are_durable() {
    let scheduler = contract_scheduler();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));

    // Bare wake (no message) inside the WaitStored gap, before registration.
    let handle = hold_gap_and(&scheduler, ParkGap::WaitStored, move |s, pid| {
        s.wake_process(pid);
    });

    let pid = scheduler
        .spawn_native(recording_factory(MARKER, &seen, &slices))
        .expect("spawn native");

    // After the lost bare wake the process parks and stays parked: the slice
    // count holds at the single spawn slice over a bounded window.
    wait_parked(&scheduler, pid);
    assert_absent(60, || slices.load(Ordering::Acquire) != 1);
    handle.join().expect("helper joins");

    // A durable marker DOES wake it — proving it was parked-alive.
    assert!(scheduler.enqueue_atom_message(pid, MARKER));
    wait_exit(&scheduler, pid);
    assert!(observed(&seen).contains(&MARKER), "durable marker observed");
    assert_eq!(slices.load(Ordering::Acquire), 2, "exactly one wake slice");
    scheduler.shutdown();
}
