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
    let (publication_phase_tx, publication_phase_rx) =
        mpsc::channel::<super::exit_events::ExitEventPublicationObserver>();

    std::thread::scope(|scope| {
        let observer_scheduler = &scheduler;
        let observer = scope.spawn(move || {
            let mut observed = HashSet::with_capacity(PROCESS_COUNT);
            for _ in 0..ROUND_COUNT {
                let publication_phase = publication_phase_rx
                    .recv_timeout(EVENT_TIMEOUT)
                    .expect("each round installs a publication gate");
                for _ in 0..PROCESSES_PER_ROUND {
                    let (pid, event_reason) = recv_exit(&subscription);
                    let (reason, _value) = observer_scheduler
                        .take_exit_outcome(pid)
                        .unwrap_or_else(|| {
                            panic!("event for pid {pid} must happen after its outcome")
                        });
                    assert!(observed.insert(pid), "exit pid {pid} published twice");
                    assert_eq!(reason, event_reason);
                }
                // Every publisher sent its event and is blocked at the test-only
                // post-send rendezvous. Release the round only after receiving
                // every event and immediately taking every corresponding outcome.
                for _ in 0..PROCESSES_PER_ROUND {
                    publication_phase.acknowledge_observed(EVENT_TIMEOUT);
                }
            }
            observed
        });

        let scheduler_ref = &scheduler;
        for _ in 0..ROUND_COUNT {
            let publication_phase = scheduler
                .shared
                .exit_tombstones
                .install_event_publication_gate();
            publication_phase_tx
                .send(publication_phase)
                .expect("observer joins each round's publication phase");
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
            // All terminal callers cross the release wall together. After each
            // successful event send, the installed test-only gate holds every
            // caller inside publication until the observer receives the whole
            // round, immediately takes each outcome, and releases the phase.
            release_wall.wait();
            for producer in producers {
                producer.join().expect("terminal producer completes");
            }
        }
        scheduler
            .shared
            .exit_tombstones
            .clear_event_publication_gate();

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

// ===== EXIT-001 walls (scheduler level) =====

/// CHARACTERIZATION (EXIT-001 R1) — green at base and green forever.
///
/// This test is a deliberate guard, not an obstacle: `subscribe_exit_events`
/// is single-use BY DESIGN because aion's singleton-drainer contract depends
/// on exactly one outcome-claiming subscription per scheduler lifetime. If a
/// change makes the second call return `Some`, that change has silently
/// deleted the guard against two drainers racing exactly-once
/// `take_exit_outcome` — treat the red as a STOP and escalate; do not
/// "fix" this test.
#[test]
fn characterization_second_exit_event_subscription_still_returns_none() {
    let scheduler = test_scheduler(1);
    let _first = scheduler
        .subscribe_exit_events()
        .expect("first subscription succeeds");
    assert!(
        scheduler.subscribe_exit_events().is_none(),
        "the exclusive outcome-claiming subscription must stay single-use"
    );
    scheduler.shutdown();
}

/// W1 — ALREADY-DEAD AT REGISTRATION: a finalized pid answers immediately
/// with its reason, from registration itself, never by blocking.
#[test]
fn w1_watch_exit_on_finalized_pid_reports_immediately_with_reason() {
    let scheduler = test_scheduler(1);
    let pid = 9_300_001;
    publish_synthetic_exit(&scheduler, pid, Term::small_int(7));
    assert!(
        scheduler.peek_exit_reason(pid).is_some(),
        "precondition: pid must really be finalized before the watch registers"
    );

    match scheduler.watch_exit(pid) {
        ExitWatchState::AlreadyExited(reason) => assert_eq!(reason, ExitReason::Normal),
        ExitWatchState::Live(watch) => {
            let outcome = watch.recv_timeout(Duration::from_secs(1));
            panic!(
                "blocked past the deadline: watch_exit on an already-finalized pid \
                 returned Live (recv gave {outcome:?}) instead of an immediate \
                 AlreadyExited(Normal)"
            );
        }
        ExitWatchState::NoRecord => {
            panic!("a finalized pid must never be reported as NoRecord")
        }
    }
    scheduler.shutdown();
}

/// R3/D4 — the unknown-pid answer is typed: no live process and no durable
/// record must be `NoRecord`, never a `Live` watch that can block forever.
#[test]
fn watch_exit_on_unknown_pid_reports_no_record_not_live() {
    let scheduler = test_scheduler(1);
    let pid = 9_310_001; // never spawned, never finalized

    match scheduler.watch_exit(pid) {
        ExitWatchState::NoRecord => {}
        ExitWatchState::Live(watch) => {
            let outcome = watch.recv_timeout(Duration::from_secs(1));
            panic!(
                "an unknown pid must answer the typed NoRecord, not arm a watch that \
                 can never fire (recv gave {outcome:?})"
            );
        }
        ExitWatchState::AlreadyExited(reason) => {
            panic!("an unknown pid reported AlreadyExited({reason:?}) with no record")
        }
    }
    scheduler.shutdown();
}

/// REVIEW POINT 2(a) at the public seam — a watch registered AFTER the
/// drainer consumed the outcome still answers with the correct reason.
#[test]
fn watch_exit_after_outcome_consumed_reports_reason() {
    let scheduler = test_scheduler(1);
    let pid = 9_320_001;
    publish_synthetic_exit(&scheduler, pid, Term::small_int(42));
    assert!(
        scheduler.take_exit_outcome(pid).is_some(),
        "precondition: the drainer consumed the outcome"
    );

    match scheduler.watch_exit(pid) {
        ExitWatchState::AlreadyExited(reason) => assert_eq!(reason, ExitReason::Normal),
        ExitWatchState::Live(watch) => {
            let outcome = watch.recv_timeout(Duration::from_secs(1));
            panic!(
                "blocked past the deadline: a consumed-outcome pid returned Live \
                 (recv gave {outcome:?}) — the token's retained reason must answer"
            );
        }
        ExitWatchState::NoRecord => {
            panic!("a finalized pid must never be reported as NoRecord after take")
        }
    }
    scheduler.shutdown();
}

/// W1 (live path) + REVIEW POINT 1 — a watch armed on a genuinely live
/// process fires on its real exit, and the woken watcher can immediately
/// take the outcome: exactly the wake-then-take sequence aion's
/// `process_event` performs before it would raise
/// `ProcessExitOutcomeMissingAfterEvent`.
#[test]
fn watch_on_live_process_fires_on_real_exit_and_outcome_is_takeable_at_wake() {
    let scheduler = test_scheduler(1);
    let pid = scheduler
        .spawn_native(Box::new(|| Box::new(WaitForTermination)))
        .expect("native process spawns");

    let watch = match scheduler.watch_exit(pid) {
        ExitWatchState::Live(watch) => watch,
        other => panic!("a live process must arm a Live watch, got {other:?}"),
    };
    scheduler.terminate_process(pid, ExitReason::Kill);

    assert_eq!(
        watch.recv_timeout(EVENT_TIMEOUT),
        Ok((pid, ExitReason::Kill)),
        "the watch must fire on the real exit"
    );
    assert!(
        scheduler.take_exit_outcome(pid).is_some(),
        "a watcher woken by a fire must find the outcome already installed"
    );
    scheduler.shutdown();
}

/// WALL 1 — race semantics ruled FINAL (Waffles 2026-07-29 16:16Z, Hermes
/// assented, lane entry 99f4e608): REPLY WINS, WITH MANDATORY POST-EXIT
/// DRAIN. Riders (Hermes, binding): the test asserts the reply's VALUE, not
/// mere arrival; the pinned interleaving is enqueue STRICTLY BEFORE kill.
///
/// This wall converts shipped-by-accident loop ordering into an asserted
/// invariant: a value enqueued before death is observable — with its value —
/// by the time the exit notification wakes the waiter, because outcome
/// installation precedes exit publication (insert, release the writer lock,
/// THEN publish).
#[test]
fn wall1_reply_enqueued_strictly_before_kill_is_observable_with_value_at_watch_wake() {
    let scheduler = test_scheduler(1);
    let pid = scheduler
        .spawn_native(Box::new(|| Box::new(WaitForTermination)))
        .expect("native process spawns");

    // The waiter exists before either racer.
    let watch = match scheduler.watch_exit(pid) {
        ExitWatchState::Live(watch) => watch,
        other => panic!("precondition: live process arms a watch, got {other:?}"),
    };

    // 1. The reply is enqueued STRICTLY BEFORE the kill (rider 2).
    scheduler
        .shared
        .exit_results
        .insert(pid, OwnedTerm::immediate(Term::small_int(1234)));

    // 2. The kill: the terminal transition captures the enqueued reply as the
    //    retained outcome, installs it, and only then publishes.
    scheduler.terminate_process(pid, ExitReason::Kill);

    // 3. The waiter wakes on the exit notification…
    assert_eq!(
        watch.recv_timeout(EVENT_TIMEOUT),
        Ok((pid, ExitReason::Kill)),
        "the waiter must wake on the exit notification"
    );

    // 4. …drains the reply path FIRST, and the reply is there WITH ITS VALUE
    //    (rider 1): success, not died-without-reply.
    let (reason, value) = scheduler.take_exit_outcome(pid).expect(
        "outcome must be installed before the exit publishes — the invariant \
         Hermes's conclusion logic rests on",
    );
    assert_eq!(reason, ExitReason::Kill);
    assert_eq!(
        value.root().as_small_int(),
        Some(1234),
        "the reply's VALUE must be observable, not a synthesized default"
    );
    scheduler.shutdown();
}

/// WALL 1b — ordering tripwire (ruled addition, Artemis 2026-07-29 17:19Z,
/// discharging the STRICT reading of Hermes's Wall-1 rider; committed Wall 1
/// above is untouched). Wall 1 certifies the value-at-wake face and is
/// deterministically green under a publish-before-install mutation:
/// `terminate_process` finalizes synchronously on the calling thread, so the
/// watcher cannot observe any intermediate state — no window exists at that
/// seam. This tripwire pins the ORDERING face instead: the killer runs on its
/// own thread, the test-only post-send publication gate parks it inside the
/// exit publication, and AT THE PARK the outcome — with its pre-kill enqueued
/// value — must already be takeable while the watch has not yet fired. Under
/// publish-before-install the park precedes the install and the at-park take
/// finds nothing: red. Structure note: nothing panics while the gate is
/// armed — observations are collected, the publisher is released and joined,
/// THEN the asserts run — so a red can never wedge the suite the way an
/// in-scope panic against a parked publisher does.
#[test]
fn wall1b_ordering_tripwire_outcome_installed_before_exit_publication() {
    let scheduler = test_scheduler(1);
    // The post-send gate only engages on a successful event send, so the
    // exclusive subscription must exist before the kill or the publisher
    // returns from publish() without ever reaching the rendezvous.
    let subscription = scheduler
        .subscribe_exit_events()
        .expect("first subscriber");
    let pid = scheduler
        .spawn_native(Box::new(|| Box::new(WaitForTermination)))
        .expect("native process spawns");

    let watch = match scheduler.watch_exit(pid) {
        ExitWatchState::Live(watch) => watch,
        other => panic!("precondition: live process arms a watch, got {other:?}"),
    };

    // The reply is enqueued STRICTLY BEFORE the kill, value face riding along.
    scheduler
        .shared
        .exit_results
        .insert(pid, OwnedTerm::immediate(Term::small_int(4321)));

    let observer = scheduler
        .shared
        .exit_tombstones
        .install_event_publication_gate();
    let (at_park_outcome, at_park_fire, wake) = std::thread::scope(|scope| {
        let killer = scope.spawn(|| scheduler.terminate_process(pid, ExitReason::Kill));

        // The killer parks inside the exit publication (post-send gate)…
        observer.wait_for_publication(EVENT_TIMEOUT);

        // …observations at the park, no asserts yet (see structure note).
        // `take_exit_outcome` reads the outcomes DashMap only; the parked
        // publisher holds `order`, so this cannot block.
        let at_park_outcome = scheduler.take_exit_outcome(pid);
        let at_park_fire = watch.recv_timeout(Duration::ZERO);

        observer.release_publication(EVENT_TIMEOUT);
        killer.join().expect("terminal caller completes");
        scheduler.shared.exit_tombstones.clear_event_publication_gate();

        let wake = watch.recv_timeout(EVENT_TIMEOUT);
        (at_park_outcome, at_park_fire, wake)
    });

    let (reason, value) = at_park_outcome.expect(
        "outcome must be takeable at the publication park — the \
         outcomes-before-exits pin Hermes's conclusion logic rests on",
    );
    assert_eq!(reason, ExitReason::Kill);
    assert_eq!(
        value.root().as_small_int(),
        Some(4321),
        "the pre-kill enqueued VALUE must be observable at the park"
    );
    assert_eq!(
        at_park_fire,
        Err(ExitEventRecvError::Timeout),
        "the watch must not fire before the exit publication returns"
    );
    assert_eq!(
        wake,
        Ok((pid, ExitReason::Kill)),
        "the waiter wakes once publication and the appended fire complete"
    );
    assert_eq!(
        subscription.recv_timeout(EVENT_TIMEOUT),
        Ok(ExitEvent::Exited {
            pid,
            reason: ExitReason::Kill,
        }),
        "the existing subscription observes the same exit"
    );
    scheduler.shutdown();
}
