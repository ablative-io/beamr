//! B-181 R3: an idle worker parks with no timeout, and worker 0 only until
//! the earliest armed timer.

use super::*;

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
