//! Receive timers: delivery, expiry in and after the park gap.

use super::*;

// ---------------------------------------------------------------------------
// Receive-timer expiry racing the wait-arm park gaps: the timer flavor of the
// lost-wakeup interleavings above. expire_timers only marks and wakes, so a
// fire that lands before wait-set registration relies entirely on the Wait
// arm's post-registration recheck noticing the mark.
// ---------------------------------------------------------------------------

/// Sleeps past the 1ms receive deadline and force-ticks the shared wheel so
/// the timer fires synchronously inside the park gap.
fn fire_receive_timer_in_gap(shared: &SharedState) {
    std::thread::sleep(std::time::Duration::from_millis(5));
    timer_integration::tick_timers(shared);
}

#[test]
fn timer_expiry_in_the_wait_park_gap_is_not_a_lost_timeout() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("timer_wait_park_gap");
    let registry = Arc::new(ModuleRegistry::new());
    let module = receive_after_module(&registry, module_name);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    // Fire the timer in the gap between the Wait arm's store-back and its
    // wait-set registration: expire_timers finds nothing in `waiting`, so
    // only the post-registration recheck (noticing the pending mark) can
    // schedule the process; the next slice consumes the mark and jumps to
    // the timeout continuation.
    let fired = Arc::new(AtomicBool::new(false));
    let fired_by_hook = Arc::clone(&fired);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap != ParkGap::WaitStored
            || pid == shared.standard_io_pid
            || fired_by_hook.swap(true, Ordering::AcqRel)
        {
            return;
        }
        fire_receive_timer_in_gap(shared);
    }));

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert!(fired.load(Ordering::Acquire), "park gap was exercised");
    let exit_value = scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root());
    assert_eq!(exit_value, Some(Term::atom(Atom::TRUE)));
    scheduler.shutdown();
}

#[test]
fn deliver_timer_pushes_message_into_target_mailbox() {
    // A `Deliver` timer (the kind backing send_after/start_timer and native
    // ctx.schedule) must deposit its message into the target's mailbox when it
    // fires. Driven deterministically with explicit instants (schedule_at +
    // tick_at), so the expired set is exact and the live scheduler thread plays
    // no part — no wall-clock sleep, no poll loop, no race on `current_tick`.
    //
    // Falsifiable: before this change `expire_timers` ONLY ran the
    // receive-timeout mark-and-wake path and never delivered
    // `ExpiredTimer.message` to any mailbox, so this assertion
    // (has_message == Some(true)) could never hold.
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let target = scheduler.spawn_test_process(false);
    let payload = Term::small_int(4242);
    assert_eq!(
        scheduler.has_message(target, payload),
        Some(false),
        "no message before the timer fires"
    );

    // Drive an isolated local wheel with explicit instants so the live
    // scheduler worker (which ticks the SHARED wheel on wall-clock) cannot race
    // the expired set. The wheel that produced the expiry is irrelevant to
    // delivery — only the recorded ExpiredTimer values matter.
    let base = std::time::Instant::now();
    let mut wheel = crate::timer::TimerWheel::new();
    let _reference = wheel.schedule_at(
        base,
        std::time::Duration::from_millis(30),
        target,
        payload,
        crate::timer::TimerKind::Deliver,
    );
    // Not yet due at +29ms; due at +30ms.
    assert!(
        wheel
            .tick_at(base + std::time::Duration::from_millis(29))
            .is_empty(),
        "the timer must not fire before its delay elapses"
    );
    let expired = wheel.tick_at(base + std::time::Duration::from_millis(30));
    assert_eq!(expired.len(), 1, "exactly one timer fires at +30ms");

    timer_integration::expire_timers_for_test(&scheduler.shared, expired);

    assert_eq!(
        scheduler.has_message(target, payload),
        Some(true),
        "a fired Deliver timer must push its message into the target mailbox"
    );
    scheduler.shutdown();
}

#[test]
fn receive_timeout_timer_does_not_deliver_a_message() {
    // Regression guard for the receive-timeout path: a ReceiveTimeout timer
    // firing must NOT push its message into the mailbox (it only marks the
    // fired-timer set for the code-position jump). Pairs with the Deliver test
    // to prove the two kinds route differently.
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let target = scheduler.spawn_test_process(false);
    let payload = Term::small_int(7);
    // Isolated local wheel (see deliver test) so the worker cannot race us.
    let base = std::time::Instant::now();
    let mut wheel = crate::timer::TimerWheel::new();
    let _reference = wheel.schedule_at(
        base,
        std::time::Duration::from_millis(1),
        target,
        payload,
        crate::timer::TimerKind::ReceiveTimeout,
    );
    let expired = wheel.tick_at(base + std::time::Duration::from_millis(1));
    assert_eq!(expired.len(), 1, "the receive-timeout timer fires");

    timer_integration::expire_timers_for_test(&scheduler.shared, expired);

    assert_eq!(
        scheduler.has_message(target, payload),
        Some(false),
        "a ReceiveTimeout timer must never deliver its message to the mailbox"
    );
    // The fired-timer mark is recorded instead (the receive-timeout path).
    assert!(
        scheduler
            .shared
            .expired_receive_timers
            .contains_key(&target),
        "a ReceiveTimeout fire records a mark for the target"
    );
    scheduler.shutdown();
}

#[test]
fn timer_expiry_after_wait_registration_schedules_the_process_once() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("timer_wait_park_gap_late");
    let registry = Arc::new(ModuleRegistry::new());
    let module = receive_after_module(&registry, module_name);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    // Fire the timer between registration and the recheck: the wake moves
    // the pid to `woken`, and the recheck's self-wake must back off (its
    // `waiting` removal finds nothing) so the process is scheduled exactly
    // once.
    let fired = Arc::new(AtomicBool::new(false));
    let fired_by_hook = Arc::clone(&fired);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap != ParkGap::WaitRegistered
            || pid == shared.standard_io_pid
            || fired_by_hook.swap(true, Ordering::AcqRel)
        {
            return;
        }
        fire_receive_timer_in_gap(shared);
    }));

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert!(fired.load(Ordering::Acquire), "park gap was exercised");
    let exit_value = scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root());
    assert_eq!(exit_value, Some(Term::atom(Atom::TRUE)));
    scheduler.shutdown();
}
