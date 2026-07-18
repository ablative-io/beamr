use std::collections::HashSet;
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

use super::exit_capture::OwnedException;
use super::exit_tombstones::TOMBSTONE_CAPACITY;
use super::*;
use crate::atom::Atom;
use crate::error::ExecError;
use crate::ets::copy::OwnedTerm;
use crate::module::ModuleRegistry;
use crate::native::native_process::{NativeContext, NativeHandler, NativeOutcome};
use crate::process::{Exception, ExitReason};
use crate::term::Term;

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

fn test_scheduler(thread_count: usize) -> Scheduler {
    Scheduler::new(
        SchedulerConfig {
            thread_count: Some(thread_count),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .expect("scheduler starts")
}

fn publish_synthetic_exit(scheduler: &Scheduler, pid: u64, value: Term) {
    scheduler
        .shared
        .exit_results
        .insert(pid, OwnedTerm::immediate(value));
    scheduler
        .shared
        .insert_exit_tombstone(pid, ExitReason::Normal);
}

fn recv_exit(subscription: &ExitEventSubscription) -> (u64, ExitReason) {
    match subscription.recv_timeout(EVENT_TIMEOUT) {
        Ok(ExitEvent::Exited { pid, reason }) => (pid, reason),
        other => panic!("expected exit event, got {other:?}"),
    }
}

#[test]
fn take_exit_outcome_is_non_blocking_and_exactly_once() {
    let scheduler = test_scheduler(1);
    let pid = 9_000_001;

    assert!(
        scheduler.take_exit_outcome(pid).is_none(),
        "none before exit"
    );
    publish_synthetic_exit(&scheduler, pid, Term::small_int(42));

    let (reason, value) = scheduler
        .take_exit_outcome(pid)
        .expect("first take succeeds");
    assert_eq!(reason, ExitReason::Normal);
    assert_eq!(value.root().as_small_int(), Some(42));
    assert!(
        scheduler.take_exit_outcome(pid).is_none(),
        "second take is empty"
    );
    let (legacy_reason, legacy_value) = scheduler.run_until_exit(pid);
    assert_eq!(legacy_reason, ExitReason::Normal);
    assert_eq!(legacy_value.root().as_small_int(), Some(42));

    let legacy_first_pid = pid + 1;
    publish_synthetic_exit(&scheduler, legacy_first_pid, Term::small_int(43));
    let (_, legacy_value) = scheduler.run_until_exit(legacy_first_pid);
    assert_eq!(legacy_value.root().as_small_int(), Some(43));
    let (_, outcome_value) = scheduler
        .take_exit_outcome(legacy_first_pid)
        .expect("legacy take does not consume additive outcome");
    assert_eq!(outcome_value.root().as_small_int(), Some(43));

    scheduler.shutdown();
}

struct WaitForTermination;

impl NativeHandler for WaitForTermination {
    fn handle(&mut self, _ctx: &mut NativeContext<'_>) -> NativeOutcome {
        NativeOutcome::Wait
    }
}

#[test]
fn receiver_contests_publication_without_misses_under_coordinated_multi_worker_churn() {
    const WORKER_COUNT: usize = 4;
    const ROUND_COUNT: usize = 100;
    const PROCESSES_PER_ROUND: usize = WORKER_COUNT;
    const PROCESS_COUNT: usize = ROUND_COUNT * PROCESSES_PER_ROUND;
    const _: () = assert!(PROCESS_COUNT < EXIT_EVENT_CAPACITY);

    let scheduler = test_scheduler(WORKER_COUNT);
    let subscription = scheduler
        .subscribe_exit_events()
        .expect("first exit subscription");
    let mut spawned = HashSet::with_capacity(PROCESS_COUNT);
    let (receiver_armed_tx, receiver_armed_rx) = mpsc::sync_channel(0);

    std::thread::scope(|scope| {
        let observer = scope.spawn(|| {
            let mut observed = HashSet::with_capacity(PROCESS_COUNT);
            // This rendezvous hands off immediately before the first blocking
            // receive. Native spawning begins only after the handoff, and every
            // terminal producer then parks at its round's release wall before it
            // can publish, putting the observer in `recv` ahead of that release.
            receiver_armed_tx
                .send(())
                .expect("spawning thread waits for receiver arm");
            for _ in 0..PROCESS_COUNT {
                let (pid, event_reason) = recv_exit(&subscription);
                let (reason, _value) = scheduler
                    .take_exit_outcome(pid)
                    .unwrap_or_else(|| panic!("event for pid {pid} must happen after its outcome"));
                assert!(observed.insert(pid), "exit pid {pid} published twice");
                assert_eq!(reason, event_reason);
            }
            observed
        });

        receiver_armed_rx
            .recv()
            .expect("receiver reaches blocking receive wall");
        let scheduler_ref = &scheduler;
        for _ in 0..ROUND_COUNT {
            let release_wall = Arc::new(Barrier::new(PROCESSES_PER_ROUND + 1));
            let mut producers = Vec::with_capacity(PROCESSES_PER_ROUND);
            for _ in 0..PROCESSES_PER_ROUND {
                let pid = scheduler
                    .spawn_native(Box::new(|| Box::new(WaitForTermination)))
                    .expect("native process spawns");
                assert!(spawned.insert(pid), "spawned pid {pid} is unique");
                let producer_wall = Arc::clone(&release_wall);
                producers.push(scope.spawn(move || {
                    producer_wall.wait();
                    scheduler_ref.terminate_process(pid, ExitReason::Normal);
                }));
            }
            // All terminal callers cross the wall together while the
            // subscriber is already draining and the scheduler's workers are
            // concurrently dispatching the newly spawned native processes.
            release_wall.wait();
            for producer in producers {
                producer.join().expect("terminal producer completes");
            }
        }

        let observed = observer.join().expect("exit observer completes");
        assert_eq!(
            observed, spawned,
            "every spawned pid publishes exactly once"
        );
    });

    scheduler.shutdown();
}

#[test]
fn durable_finalization_survives_take_and_untaken_outcome_tombstone_eviction() {
    let scheduler = test_scheduler(1);
    let subscription = scheduler
        .subscribe_exit_events()
        .expect("first exit subscription");
    let taken_pid = 10_000_000;
    let retained_pid = taken_pid + 1;

    publish_synthetic_exit(&scheduler, taken_pid, Term::small_int(6));
    assert_eq!(recv_exit(&subscription).0, taken_pid);
    assert!(scheduler.take_exit_outcome(taken_pid).is_some());
    publish_synthetic_exit(&scheduler, retained_pid, Term::small_int(7));
    assert_eq!(recv_exit(&subscription).0, retained_pid);

    for offset in 2..=(TOMBSTONE_CAPACITY as u64 + 1) {
        let pid = taken_pid + offset;
        publish_synthetic_exit(&scheduler, pid, Term::small_int(offset as i64));
        assert_eq!(recv_exit(&subscription).0, pid);
        assert!(
            scheduler.take_exit_outcome(pid).is_some(),
            "a take immediately following a delivered event must not miss"
        );
    }

    assert_eq!(scheduler.peek_exit_reason(taken_pid), None);
    assert_eq!(scheduler.peek_exit_reason(retained_pid), None);
    scheduler.terminate_process(taken_pid, ExitReason::Kill);
    scheduler.terminate_process(retained_pid, ExitReason::Kill);

    assert!(
        scheduler.take_exit_outcome(taken_pid).is_none(),
        "a taken outcome cannot be re-armed after tombstone eviction"
    );
    let (reason, value) = scheduler
        .take_exit_outcome(retained_pid)
        .expect("duplicate cleanup preserves the original untaken outcome");
    assert_eq!(reason, ExitReason::Normal);
    assert_eq!(value.root().as_small_int(), Some(7));
    assert_eq!(
        subscription.recv_timeout(Duration::ZERO),
        Err(ExitEventRecvError::Timeout),
        "duplicate cleanup cannot publish a second event"
    );

    scheduler.shutdown();
}

#[test]
fn event_queue_overflow_is_typed_and_outcomes_remain_recoverable() {
    let scheduler = test_scheduler(1);
    let subscription = scheduler
        .subscribe_exit_events()
        .expect("first exit subscription");
    let pids: Vec<_> = (0..=EXIT_EVENT_CAPACITY as u64)
        .map(|offset| 20_000_000 + offset)
        .collect();

    for &pid in &pids {
        publish_synthetic_exit(&scheduler, pid, Term::small_int(pid as i64));
    }

    assert_eq!(
        subscription.recv_timeout(EVENT_TIMEOUT),
        Ok(ExitEvent::Lagged),
        "overflow must be visible rather than silently dropping an event"
    );
    for pid in pids {
        assert!(
            scheduler.take_exit_outcome(pid).is_some(),
            "lag recovery can scan every tracked pid"
        );
    }

    scheduler.shutdown();
}

fn install_diagnostics(scheduler: &Scheduler, pid: u64) {
    scheduler.shared.exit_errors.insert(pid, ExecError::Badarg);
    scheduler.shared.exit_exceptions.insert(
        pid,
        OwnedException::capture_with_frames(
            Exception {
                class: Term::atom(Atom::ERROR),
                reason: Term::atom(Atom::BADARG),
                stacktrace: Term::NIL,
            },
            Vec::new(),
        ),
    );
    publish_synthetic_exit(scheduler, pid, Term::small_int(99));
}

#[test]
fn outcome_and_error_exception_diagnostics_consume_independently() {
    let scheduler = test_scheduler(1);
    let outcome_first = 30_000_001;
    let diagnostics_first = 30_000_002;
    install_diagnostics(&scheduler, outcome_first);
    install_diagnostics(&scheduler, diagnostics_first);

    assert!(scheduler.take_exit_outcome(outcome_first).is_some());
    assert_eq!(
        scheduler.take_exit_error(outcome_first),
        Some(ExecError::Badarg)
    );
    let exception = scheduler
        .take_exit_exception(outcome_first)
        .expect("exception survives outcome take");
    assert_eq!(exception.view().class, Term::atom(Atom::ERROR));

    assert_eq!(
        scheduler.take_exit_error(diagnostics_first),
        Some(ExecError::Badarg)
    );
    assert!(scheduler.take_exit_exception(diagnostics_first).is_some());
    let (_, value) = scheduler
        .take_exit_outcome(diagnostics_first)
        .expect("outcome survives diagnostic takes");
    assert_eq!(value.root().as_small_int(), Some(99));

    scheduler.shutdown();
}
