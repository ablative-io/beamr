//! The timer wheel's unit tests.

use std::time::{Duration, Instant};

use super::{TimerKind, TimerWheel};
use crate::atom::Atom;
use crate::term::Term;

#[test]
fn timer_schedule_and_tick_expire_due_timers() {
    let start = Instant::now();
    let mut wheel = TimerWheel::with_bucket_count(8);
    let reference = wheel.schedule_at(
        start,
        Duration::from_millis(10),
        12,
        Term::atom(Atom::OK),
        TimerKind::Deliver,
    );

    assert!(wheel.tick_at(start + Duration::from_millis(9)).is_empty());
    let expired = wheel.tick_at(start + Duration::from_millis(10));

    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].reference, reference);
    assert_eq!(expired[0].target_pid, 12);
    assert_eq!(expired[0].message, Term::atom(Atom::OK));
    assert_eq!(expired[0].kind, TimerKind::Deliver);
    assert!(wheel.is_empty());
}

#[test]
fn timer_reports_earliest_absolute_deadline() {
    let start = Instant::now();
    let mut wheel = TimerWheel::with_bucket_count(8);
    let later = start + Duration::from_millis(40);
    let earlier = start + Duration::from_millis(10);

    wheel.schedule_at(
        start,
        Duration::from_millis(40),
        1,
        Term::small_int(1),
        TimerKind::Deliver,
    );
    let earlier_ref = wheel.schedule_at(
        start,
        Duration::from_millis(10),
        2,
        Term::small_int(2),
        TimerKind::Deliver,
    );

    assert_eq!(wheel.earliest_deadline(), Some(earlier));
    assert_eq!(
        wheel.cancel_at(earlier_ref, start),
        Some(Duration::from_millis(10))
    );
    assert_eq!(wheel.earliest_deadline(), Some(later));
    let _expired = wheel.tick_at(later);
    assert_eq!(wheel.earliest_deadline(), None);
}

#[test]
fn timer_cancellation_is_constant_time_and_returns_remaining_time() {
    let start = Instant::now();
    let mut wheel = TimerWheel::with_bucket_count(4);
    let reference = wheel.schedule_at(
        start,
        Duration::from_millis(100),
        1,
        Term::small_int(1),
        TimerKind::ReceiveTimeout,
    );

    assert_eq!(
        wheel.cancel_at(reference, start + Duration::from_millis(40)),
        Some(Duration::from_millis(60))
    );
    assert_eq!(wheel.cancel_at(reference, start), None);
    assert!(wheel.tick_at(start + Duration::from_millis(100)).is_empty());
}

#[test]
fn timer_cancellation_after_fire_returns_none() {
    let start = Instant::now();
    let mut wheel = TimerWheel::with_bucket_count(4);
    let reference = wheel.schedule_at(
        start,
        Duration::from_millis(1),
        1,
        Term::small_int(1),
        TimerKind::Deliver,
    );

    assert_eq!(wheel.tick_at(start + Duration::from_millis(1)).len(), 1);
    assert_eq!(
        wheel.cancel_at(reference, start + Duration::from_millis(1)),
        None
    );
}

#[test]
fn timer_reserved_reference_cannot_be_scheduled_twice() {
    let start = Instant::now();
    let mut wheel = TimerWheel::with_bucket_count(4);
    let reference = wheel.reserve_reference();

    assert_eq!(
        wheel.schedule_reserved_at(
            reference,
            start,
            Duration::from_millis(10),
            1,
            Term::small_int(1),
            TimerKind::Deliver,
        ),
        Some(reference)
    );
    assert_eq!(
        wheel.schedule_reserved_at(
            reference,
            start,
            Duration::from_millis(20),
            1,
            Term::small_int(2),
            TimerKind::Deliver,
        ),
        None
    );

    let expired = wheel.tick_at(start + Duration::from_millis(20));
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].message, Term::small_int(1));
    assert!(wheel.is_empty());
}

#[test]
fn timer_handles_ten_thousand_concurrent_timers() {
    let start = Instant::now();
    let mut wheel = TimerWheel::with_bucket_count(256);
    for index in 0..10_000 {
        wheel.schedule_at(
            start,
            Duration::from_millis(index % 100),
            index,
            Term::small_int(index as i64),
            TimerKind::Deliver,
        );
    }

    assert_eq!(wheel.len(), 10_000);
    let expired = wheel.tick_at(start + Duration::from_millis(100));
    assert_eq!(expired.len(), 10_000);
    assert!(wheel.is_empty());
}
