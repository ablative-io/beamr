use std::collections::HashSet;
use std::sync::Arc;
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

struct StopImmediately;

impl NativeHandler for StopImmediately {
    fn handle(&mut self, _ctx: &mut NativeContext<'_>) -> NativeOutcome {
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

#[test]
fn exit_events_wake_and_take_without_misses_under_concurrent_process_exits() {
    const PROCESS_COUNT: usize = 256;

    let scheduler = test_scheduler(4);
    let subscription = scheduler
        .subscribe_exit_events()
        .expect("first exit subscription");
    let mut outstanding = HashSet::with_capacity(PROCESS_COUNT);
    for _ in 0..PROCESS_COUNT {
        let pid = scheduler
            .spawn_native(Box::new(|| Box::new(StopImmediately)))
            .expect("native process spawns");
        assert!(outstanding.insert(pid));
    }

    for _ in 0..PROCESS_COUNT {
        let (pid, event_reason) = recv_exit(&subscription);
        assert!(outstanding.remove(&pid), "event pid is unique and spawned");
        let (reason, _value) = scheduler
            .take_exit_outcome(pid)
            .unwrap_or_else(|| panic!("event for pid {pid} must happen after its outcome"));
        assert_eq!(reason, event_reason);
    }
    assert!(
        outstanding.is_empty(),
        "every spawned process produced an event"
    );

    scheduler.shutdown();
}

#[test]
fn delivered_outcomes_survive_legacy_tombstone_churn_until_taken() {
    let scheduler = test_scheduler(1);
    let subscription = scheduler
        .subscribe_exit_events()
        .expect("first exit subscription");
    let retained_pid = 10_000_000;

    publish_synthetic_exit(&scheduler, retained_pid, Term::small_int(7));
    assert_eq!(recv_exit(&subscription).0, retained_pid);

    for offset in 1..=(TOMBSTONE_CAPACITY as u64 + 1) {
        let pid = retained_pid + offset;
        publish_synthetic_exit(&scheduler, pid, Term::small_int(offset as i64));
        assert_eq!(recv_exit(&subscription).0, pid);
        assert!(
            scheduler.take_exit_outcome(pid).is_some(),
            "a take immediately following a delivered event must not miss"
        );
    }

    assert_eq!(scheduler.peek_exit_reason(retained_pid), None);
    let (reason, value) = scheduler
        .take_exit_outcome(retained_pid)
        .expect("untaken outcome survives tombstone eviction");
    assert_eq!(reason, ExitReason::Normal);
    assert_eq!(value.root().as_small_int(), Some(7));

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
