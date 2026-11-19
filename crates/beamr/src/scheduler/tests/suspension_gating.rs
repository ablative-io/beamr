//! Result-gated suspensions: a stale or foreign completion, a message or a
//! termination never re-executes or resurrects a gated await.

use super::*;

// ---------------------------------------------------------------------------
// Call-identity-gated suspension protocol: results are keyed by (pid, call
// id) and applied only at the suspension that produced them; gated host
// awaits are never woken (or re-executed) by plain messages; resumes are
// identity-gated and sticky.
// ---------------------------------------------------------------------------

// --- Defect 1: the query-re-entry shape. A timed gated await times out and
// re-submits under a new call id; the ORIGINAL call's late result must be
// dropped, and only the new call's result resumes the process. ---

static REENTRY_RUNS: AtomicUsize = AtomicUsize::new(0);
static REENTRY_FIRST_ID: AtomicU64 = AtomicU64::new(0);
static REENTRY_SECOND_ID: AtomicU64 = AtomicU64::new(0);

fn reentry_timed_await_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    match REENTRY_RUNS.fetch_add(1, Ordering::AcqRel) {
        0 => {
            let call_id = context
                .request_await_suspend(Some(40))
                .expect("attached process allocates a call id");
            REENTRY_FIRST_ID.store(call_id, Ordering::Release);
            Ok(Term::NIL)
        }
        1 => {
            // Timeout re-entry: the protocol hands the timeout to the
            // native, which clears it and re-submits under a NEW call id.
            assert!(
                context.receive_timeout_expired(),
                "second run must be the timeout re-entry"
            );
            context.clear_receive_timeout();
            let call_id = context
                .request_await_suspend(None)
                .expect("attached process allocates a call id");
            REENTRY_SECOND_ID.store(call_id, Ordering::Release);
            Ok(Term::NIL)
        }
        _ => {
            // A third execution means a stale completion re-executed the
            // await: the exact double-submit defect. Make it visible in the
            // exit value.
            Ok(Term::small_int(666))
        }
    }
}

#[test]
fn stale_result_for_a_superseded_await_is_dropped_not_applied() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("reentry_await");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, reentry_timed_await_native, None);
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    // Let the await time out and re-submit under a new call id.
    wait_until(10_000, || REENTRY_SECOND_ID.load(Ordering::Acquire) != 0);
    let first_id = REENTRY_FIRST_ID.load(Ordering::Acquire);
    let second_id = REENTRY_SECOND_ID.load(Ordering::Acquire);
    assert!(first_id != 0 && second_id > first_id);

    // The ORIGINAL call's result arrives late: it must be refused at
    // publish time and the process must stay parked.
    assert!(
        !scheduler.wake_with_result_for(pid, first_id, Term::small_int(1)),
        "stale completion must be refused"
    );
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        !scheduler.shared.exit_tombstones.contains_key(&pid),
        "stale completion must not resume the process"
    );
    assert_eq!(REENTRY_RUNS.load(Ordering::Acquire), 2);

    // Only the current call's result resumes the process — applied exactly
    // once, at the suspension that produced it.
    assert!(scheduler.wake_with_result_for(pid, second_id, Term::small_int(2)));
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert_eq!(exit_value(&scheduler, pid), Some(Term::small_int(2)));
    assert_eq!(
        REENTRY_RUNS.load(Ordering::Acquire),
        2,
        "the result resumed the process without re-executing the await"
    );
    scheduler.shutdown();
}

// --- External kill while the victim is checked out (Executing slot): the
// finalizer's table token is consumed while a worker owns the real body, so
// the body half must be finished by that worker's store-back — never by
// re-inserting a body for the dead pid. ---

static EXEC_TERMINATE_ENTERED: AtomicUsize = AtomicUsize::new(0);
static EXEC_TERMINATE_RELEASE: AtomicBool = AtomicBool::new(false);

/// Signals entry, then blocks until the test releases it — pinning the
/// process in the `Executing` slot state for a deterministic window.
fn exec_terminate_blocking_native(
    _args: &[Term],
    _context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    EXEC_TERMINATE_ENTERED.store(1, Ordering::Release);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !EXEC_TERMINATE_RELEASE.load(Ordering::Acquire) {
        assert!(
            std::time::Instant::now() <= deadline,
            "release signal timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Ok(Term::NIL)
}

/// `terminate_process` against a mid-slice process: external finalization
/// removes the `Executing` shadow and spends the table token while the
/// worker still owns the real `Process`. The worker's store-back must then
/// DISPOSE of that body — re-inserting it (the pre-fix behavior) resurrected
/// a body no later finalizer would ever reap, and `enqueue_atom_message`
/// kept delivering into it despite the dead pid (a C3 violation).
#[test]
fn terminate_while_executing_finalizes_without_resurrecting_the_body() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("exec_terminate");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, exec_terminate_blocking_native, None);
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || {
        EXEC_TERMINATE_ENTERED.load(Ordering::Acquire) == 1
    });

    // The worker is inside the native: the slot is an Executing shadow.
    scheduler.terminate_process(pid, ExitReason::Killed);

    // External finalization ran to completion against the shadow: table
    // token spent, shadow removed, while the worker still runs the slice.
    assert!(scheduler.shared.process_table.get(pid).is_none());
    assert!(!scheduler.shared.process_bodies.contains_key(&pid));

    // Release the native, then JOIN the worker: shutdown is the barrier that
    // guarantees the store-back has fully run before the assertions below.
    EXEC_TERMINATE_RELEASE.store(true, Ordering::Release);
    scheduler.shutdown();

    assert!(
        !scheduler.shared.process_bodies.contains_key(&pid),
        "store-back after external finalization must dispose of the checked-out body, not resurrect it"
    );
    assert!(scheduler.shared.exit_tombstones.contains_key(&pid));
    // Delivery keys off process_bodies alone, so this is the resurrection
    // probe: a re-inserted body would accept the message for a dead pid.
    assert!(
        !scheduler.enqueue_atom_message(pid, Atom::OK),
        "a dead pid must refuse delivery after its body is reaped"
    );
}

// --- Defect 2: a gated host await has a wake guard — a plain message
// delivery must neither wake the process nor re-execute the await native.
// ---

static GUARD_RUNS: AtomicUsize = AtomicUsize::new(0);
static GUARD_PARKED: AtomicBool = AtomicBool::new(false);

fn guarded_await_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    GUARD_RUNS.fetch_add(1, Ordering::AcqRel);
    let _call_id = context.request_await_suspend(None);
    GUARD_PARKED.store(true, Ordering::Release);
    Ok(Term::NIL)
}

#[test]
fn message_delivery_does_not_wake_or_reexecute_a_gated_await() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("guarded_await");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, guarded_await_native, None);
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || GUARD_PARKED.load(Ordering::Acquire));
    // Ensure the park completed (slot stored back as Present).
    wait_until(10_000, || scheduler.trap_exit(pid).is_some());

    assert!(scheduler.enqueue_atom_message(pid, Atom::OK));
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(
        GUARD_RUNS.load(Ordering::Acquire),
        1,
        "a message wake re-executed the gated await native"
    );
    assert!(!scheduler.shared.exit_tombstones.contains_key(&pid));

    // The pid-resolved embedder seam completes the await; the native is NOT
    // re-executed — the result lands in x0 past the call instruction.
    assert!(scheduler.wake_with_result(pid, Term::small_int(42)));
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert_eq!(exit_value(&scheduler, pid), Some(Term::small_int(42)));
    assert_eq!(GUARD_RUNS.load(Ordering::Acquire), 1);
    scheduler.shutdown();
}

// --- Defect 1/3 cross-protocol: a host result published at a process parked
// for a DIRTY call must be refused (kind mismatch), and identity-gated
// resume must refuse to flip an in-flight dirty call. ---

static CROSS_DIRTY_RELEASE: AtomicBool = AtomicBool::new(false);
static CROSS_DIRTY_RUNS: AtomicUsize = AtomicUsize::new(0);

fn cross_dirty_native(
    _args: &[Term],
    _context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    CROSS_DIRTY_RUNS.fetch_add(1, Ordering::AcqRel);
    while !CROSS_DIRTY_RELEASE.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    Ok(Term::small_int(7))
}

#[test]
fn host_results_and_resumes_cannot_touch_an_in_flight_dirty_call() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("cross_dirty");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(
        &registry,
        module_name,
        cross_dirty_native,
        Some(crate::scheduler::DirtySchedulerKind::Cpu),
    );
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || CROSS_DIRTY_RUNS.load(Ordering::Acquire) == 1);
    wait_until(10_000, || scheduler.trap_exit(pid).is_some());

    // Wrong-kind publish: refused, dropped.
    assert!(
        !scheduler.wake_with_result(pid, Term::small_int(1)),
        "a host result must not target a dirty-call suspension"
    );
    // Identity-gated resume: refused (only the dirty completion may resume).
    assert!(
        !scheduler.resume_process(pid),
        "resume_process must not flip an in-flight dirty call"
    );
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(CROSS_DIRTY_RUNS.load(Ordering::Acquire), 1);
    assert!(!scheduler.shared.exit_tombstones.contains_key(&pid));

    CROSS_DIRTY_RELEASE.store(true, Ordering::Release);
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert_eq!(exit_value(&scheduler, pid), Some(Term::small_int(7)));
    assert_eq!(
        CROSS_DIRTY_RUNS.load(Ordering::Acquire),
        1,
        "the dirty call was double-submitted"
    );
    scheduler.shutdown();
}

// --- Pre-park publish: a completion published while the requesting slice is
// still Executing (the mirror is registered at request time) must be found
// by the Wait arm's recheck — no lost wakeup. ---

static PREPARK_ID: AtomicU64 = AtomicU64::new(0);
static PREPARK_PUBLISHED: AtomicBool = AtomicBool::new(false);

fn prepark_await_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    let call_id = context
        .request_await_suspend(None)
        .expect("attached process allocates a call id");
    PREPARK_ID.store(call_id, Ordering::Release);
    // Hold the slice open until the test has published the completion: the
    // park sequence must then self-wake on the pending result.
    while !PREPARK_PUBLISHED.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    Ok(Term::NIL)
}

#[test]
fn completion_published_while_executing_is_not_a_lost_wakeup() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("prepark_await");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, prepark_await_native, None);
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || PREPARK_ID.load(Ordering::Acquire) != 0);
    let call_id = PREPARK_ID.load(Ordering::Acquire);
    // The slot is Executing: the mirror published by request_await_suspend
    // must already accept the completion.
    assert!(scheduler.wake_with_result_for(pid, call_id, Term::small_int(11)));
    PREPARK_PUBLISHED.store(true, Ordering::Release);

    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert_eq!(exit_value(&scheduler, pid), Some(Term::small_int(11)));
    scheduler.shutdown();
}

// --- Defect 4: an embedder resume racing the hook suspension's park gap is
// sticky — recorded against the suspension and consumed at the next slice,
// never lost. ---

#[test]
fn hook_resume_in_the_suspend_park_gap_is_sticky_not_lost() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("hook_gap_loop");
    let registry = Arc::new(ModuleRegistry::new());
    // Entry is after func_info (which now RAISES on the no-match fail edge), so
    // the happy path is the self-tail-call loop only.
    let module = registry.insert(test_module(
        module_name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::CallOnly {
                arity: Operand::Unsigned(0),
                label: Operand::Label(1),
            },
        ],
    ));
    let scheduler = Arc::new(single_thread_scheduler(&registry));
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let hook_calls_by_hook = Arc::clone(&hook_calls);
    scheduler.hook().register(move |_event| {
        if hook_calls_by_hook.fetch_add(1, Ordering::AcqRel) == 0 {
            HookDecision::Suspend
        } else {
            HookDecision::Continue
        }
    });

    // A helper thread issues resume_process exactly when the park gap hook
    // fires (slot stored back, wait-set registration not yet done): the
    // window where the old protocol lost the resume.
    let (gap_tx, gap_rx) = std::sync::mpsc::channel::<u64>();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<bool>();
    let gap_tx = Mutex::new(gap_tx);
    let done_rx = Mutex::new(done_rx);
    let resumer_scheduler = Arc::clone(&scheduler);
    let resumer = std::thread::spawn(move || {
        if let Ok(pid) = gap_rx.recv() {
            let resumed = resumer_scheduler.resume_process(pid);
            let _ = done_tx.send(resumed);
        }
    });
    let fired = Arc::new(AtomicBool::new(false));
    let fired_by_hook = Arc::clone(&fired);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap != ParkGap::SuspendStored
            || pid == shared.standard_io_pid
            || fired_by_hook.swap(true, Ordering::AcqRel)
        {
            return;
        }
        // Run the embedder resume synchronously inside the gap.
        let _ = lock_or_recover(&gap_tx).send(pid);
        // The resume cannot unpark a pid that is not registered yet; what
        // matters is that it is recorded (sticky) and consumed below.
        let _resumed_in_gap = lock_or_recover(&done_rx).recv();
    }));

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    // The sticky resume must let the process run again (hook called at
    // least twice) instead of sleeping forever.
    wait_until(10_000, || hook_calls.load(Ordering::Acquire) >= 2);
    assert!(fired.load(Ordering::Acquire), "park gap was exercised");
    scheduler
        .shared
        .exit_tombstones
        .insert(pid, ExitReason::Kill);
    scheduler.shutdown();
    resumer.join().expect("resumer thread");
}
