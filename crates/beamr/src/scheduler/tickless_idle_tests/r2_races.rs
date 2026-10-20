//! B-181 R2: no wake is lost between a worker's checks and its park; the
//! races and the idle-gap interleavings, each failing on the loss detector's
//! named report.

use super::*;

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
