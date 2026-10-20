//! The review conditions of EMBEDDER-COMPOSITION-SPEC §3.8.2: C1's surplus
//! edge and idle count, C4's one-lap catch-up, C2's arm edge, and F1's
//! deadline read that does not grow with the armed timers.

use super::*;
use crate::timer::TimerRef;

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

/// C1's second red: after a run in which workers find work without
/// parking, the idle count equals the number of parked workers. A count
/// left raised on the find-work path would read above it.
#[test]
fn idle_count_equals_parked_workers_after_work_found_without_parking() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let module = return_module(&registry, atoms.intern("tickless_idle_count"));
    let scheduler = scheduler_with_workers(&registry, 4);
    let workers = scheduler.shared.thread_count;
    let pids: Vec<u64> = (0..1_000)
        .map(|_| {
            scheduler
                .spawn_process(&module)
                .unwrap_or_else(|error| panic!("spawn: {error}"))
        })
        .collect();
    for pid in &pids {
        assert_eq!(await_exit(&scheduler, *pid), ExitReason::Normal);
    }
    let events = waiting_events(&scheduler);
    // Every worker parks once the work is done; the herd from one more spawn
    // makes each that already parked park again, so each reports.
    let last = scheduler
        .spawn_process(&module)
        .unwrap_or_else(|error| panic!("spawn last: {error}"));
    assert_eq!(await_exit(&scheduler, last), ExitReason::Normal);
    loop {
        let (_worker, timeout) = events
            .recv()
            .unwrap_or_else(|error| panic!("waiting events: {error}"));
        assert_eq!(
            timeout, None,
            "an idle worker with no timer parks unbounded"
        );
        let wait_set = lock_or_recover(&scheduler.shared.wait_set);
        let parked = scheduler.parked_worker_count();
        if parked == workers {
            assert_eq!(scheduler.idle_worker_count(), parked);
            drop(wait_set);
            break;
        }
    }
    scheduler.shutdown();
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

/// F1's red, from Hermes's reading of c60b248d: the deadline worker 0 reads
/// and publishes at every park (`publish_park`, under the timers lock, which
/// every arm and cancel on the other workers waits for) costs the same with
/// 10 armed timers as with 10,000. Counted through the wheel's test-only
/// count of the entries the read examines, never by timing.
#[test]
fn the_parked_deadline_read_stays_flat_as_armed_timers_grow() {
    const SIZES: [usize; 3] = [10, 1_000, 10_000];
    const FIRST: Duration = Duration::from_millis(1_000);
    let start = Instant::now();
    let reads: Vec<u64> = SIZES
        .iter()
        .map(|&count| {
            let mut wheel = TimerWheel::new();
            let armed: Vec<TimerRef> = (0..count)
                .map(|index| {
                    let offset = Duration::from_millis(u64::try_from(index).unwrap_or(u64::MAX));
                    wheel.schedule_at(
                        start,
                        FIRST + offset,
                        7,
                        Term::atom(Atom::OK),
                        TimerKind::Deliver,
                    )
                })
                .collect();
            let before = wheel.deadline_reads();
            assert_eq!(wheel.publish_park(), Some(start + FIRST));
            let read = wheel.deadline_reads() - before;
            // The cancel half of the index (Hermes, 21 Oct): once the
            // earliest of N is cancelled, the earliest deadline is the next.
            assert!(
                wheel.cancel_at(armed[0], start).is_some(),
                "the earliest of {count} armed timers was not pending to cancel"
            );
            assert_eq!(
                wheel.earliest_deadline(),
                Some(start + FIRST + Duration::from_millis(1)),
                "after cancelling the earliest of {count} armed timers, the earliest \
                 deadline must be the next one's"
            );
            read
        })
        .collect();
    assert!(
        reads.iter().all(|read| *read == reads[0]),
        "worker 0's park read {reads:?} timer entries with {SIZES:?} timers armed: \
         the read must not grow with the number of armed timers"
    );
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
