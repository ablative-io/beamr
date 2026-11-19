//! peek_exit_reason and run_until_exit under tombstone pressure.

use super::*;

// --- peek_exit_reason: non-blocking, non-consuming read of a dead process's
// exit reason. Exit tombstones are written once at teardown and never removed
// for the scheduler's lifetime, so a post-exit peek reliably observes the
// reason even for an externally terminated process. ---

static PEEK_PARKED: AtomicBool = AtomicBool::new(false);

fn peek_park_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    let _call_id = context.request_await_suspend(None);
    PEEK_PARKED.store(true, Ordering::Release);
    Ok(Term::NIL)
}

#[test]
fn peek_exit_reason_returns_none_for_live_and_unknown_pids() {
    PEEK_PARKED.store(false, Ordering::Release);
    let atoms = AtomTable::new();
    let module_name = atoms.intern("peek_park_live");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, peek_park_native, None);
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    // Wait until the process has parked (live, no tombstone).
    wait_until(10_000, || PEEK_PARKED.load(Ordering::Acquire));
    wait_until(10_000, || scheduler.trap_exit(pid).is_some());

    assert_eq!(
        scheduler.peek_exit_reason(pid),
        None,
        "a live, parked process has no exit tombstone"
    );
    // A pid that was never spawned.
    assert_eq!(
        scheduler.peek_exit_reason(u64::MAX),
        None,
        "a never-spawned pid has no exit tombstone"
    );

    scheduler.shutdown();
}

#[test]
fn peek_exit_reason_observes_external_termination_without_consuming() {
    PEEK_PARKED.store(false, Ordering::Release);
    let atoms = AtomTable::new();
    let module_name = atoms.intern("peek_park_kill");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, peek_park_native, None);
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || PEEK_PARKED.load(Ordering::Acquire));
    wait_until(10_000, || scheduler.trap_exit(pid).is_some());
    assert_eq!(scheduler.peek_exit_reason(pid), None, "live before kill");

    // Terminate externally (scheduler-kill shape): writes the tombstone.
    scheduler.terminate_process(pid, ExitReason::Kill);
    wait_until(10_000, || scheduler.peek_exit_reason(pid).is_some());

    // The reason is the one the external kill recorded.
    assert_eq!(scheduler.peek_exit_reason(pid), Some(ExitReason::Kill));
    // Non-consuming: a second peek still observes the same reason.
    assert_eq!(
        scheduler.peek_exit_reason(pid),
        Some(ExitReason::Kill),
        "peek must not remove the tombstone"
    );
    assert!(
        scheduler.shared.exit_tombstones.contains_key(&pid),
        "tombstone must survive peeking"
    );
    // The existing blocking reader still works after peeking.
    let (reason, _value) = scheduler
        .run_until_exit(pid)
        .unwrap_or_else(|failure| panic!("scheduler failed: {failure}"));
    assert_eq!(reason, ExitReason::Kill);
    // And peek still works after run_until_exit (which leaves the tombstone).
    assert_eq!(scheduler.peek_exit_reason(pid), Some(ExitReason::Kill));

    scheduler.shutdown();
}

/// (d) `run_until_exit` returns the correct reason for a process that just
/// exited even under bounded-tombstone cap pressure.
///
/// The bounded store evicts in insertion order (FIFO). The pid we wait on has
/// its tombstone written *last* (it is the newest entry), so FIFO eviction —
/// which only ever reclaims the OLDEST entries — can never remove it out from
/// under a blocked reader. Here we saturate the store with `TOMBSTONE_CAPACITY`
/// synthetic older tombstones first, then externally terminate the real
/// process; its tombstone becomes the newest, the synthetic ones are evicted
/// instead, and `run_until_exit` still returns the right reason.
#[test]
fn run_until_exit_correct_under_tombstone_cap_pressure() {
    use super::exit_tombstones::TOMBSTONE_CAPACITY;

    PEEK_PARKED.store(false, Ordering::Release);
    let atoms = AtomTable::new();
    let module_name = atoms.intern("run_until_exit_cap_pressure");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, peek_park_native, None);
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || PEEK_PARKED.load(Ordering::Acquire));
    wait_until(10_000, || scheduler.trap_exit(pid).is_some());

    // Saturate the bounded store with synthetic OLDER tombstones using a pid
    // range that cannot collide with the live process's pid. After this the
    // store is at capacity and the live pid is still absent from it.
    let synthetic_base = pid + 1_000_000;
    for offset in 0..(TOMBSTONE_CAPACITY as u64) {
        scheduler
            .shared
            .insert_exit_tombstone(synthetic_base + offset, ExitReason::Normal);
    }
    assert_eq!(
        scheduler.peek_exit_reason(pid),
        None,
        "live process has no tombstone yet, even with the store saturated"
    );

    // Externally terminate the real process: its tombstone is now the NEWEST
    // entry. This pushes the store over capacity, evicting the OLDEST synthetic
    // tombstone — never the just-written one for `pid`.
    scheduler.terminate_process(pid, ExitReason::Kill);

    // The blocking reader observes the real exit and returns the right reason,
    // despite full cap pressure.
    let (reason, _value) = scheduler
        .run_until_exit(pid)
        .unwrap_or_else(|failure| panic!("scheduler failed: {failure}"));
    assert_eq!(
        reason,
        ExitReason::Kill,
        "run_until_exit must return the real exit reason under cap pressure"
    );
    // The oldest synthetic tombstone was evicted to make room for the newest.
    assert_eq!(
        scheduler.peek_exit_reason(synthetic_base),
        None,
        "the oldest synthetic tombstone is the one evicted, not the live pid's"
    );

    scheduler.shutdown();
}
