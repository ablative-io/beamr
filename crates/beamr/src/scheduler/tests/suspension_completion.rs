//! Completing suspensions: exit purges suspension state, results race
//! timeouts and stale publishers, and dirty natives re-suspend or trampoline.

use super::*;

// --- Defect 5: process exit purges every (pid, *) suspension structure. ---

static PURGE_PARKED: AtomicBool = AtomicBool::new(false);

fn purge_await_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    let _call_id = context.request_await_suspend(None);
    PURGE_PARKED.store(true, Ordering::Release);
    Ok(Term::NIL)
}

#[test]
fn cleanup_exited_process_purges_all_suspension_state() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("purge_await");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, purge_await_native, None);
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || PURGE_PARKED.load(Ordering::Acquire));
    wait_until(10_000, || scheduler.trap_exit(pid).is_some());
    assert!(scheduler.shared.suspensions.contains_key(&pid));

    // Leave a published-but-unconsumed completion behind, then kill the
    // process externally: everything keyed by the pid must be purged.
    assert!(scheduler.wake_with_result(pid, Term::small_int(5)));
    scheduler.terminate_process(pid, ExitReason::Kill);
    wait_until(10_000, || {
        !scheduler.shared.suspensions.contains_key(&pid)
            && !scheduler.shared.suspension_results.contains_key(&pid)
    });
    assert!(!scheduler.shared.pending_resumes.contains_key(&pid));
    assert!(!scheduler.shared.file_io_results.contains_key(&pid));
    assert!(!scheduler.shared.expired_receive_timers.contains_key(&pid));
    scheduler.shutdown();
}

// --- Defect 8 (result-vs-timeout race): when a completion and the timeout
// fire together, the completion wins, the timeout metadata is fully cleared
// (no bogus timer can re-arm at the stale position), and the native never
// re-runs. ---

static RACE_RUNS: AtomicUsize = AtomicUsize::new(0);
static RACE_ID: AtomicU64 = AtomicU64::new(0);

fn race_timed_await_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    RACE_RUNS.fetch_add(1, Ordering::AcqRel);
    let call_id = context
        .request_await_suspend(Some(1))
        .expect("attached process allocates a call id");
    RACE_ID.store(call_id, Ordering::Release);
    Ok(Term::NIL)
}

#[test]
fn result_beats_timeout_and_clears_the_timed_await_metadata() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("race_timed_await");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, race_timed_await_native, None);
    let scheduler = single_thread_scheduler(&registry);

    // In the WaitStored park gap (timer registered, pid not yet in the wait
    // set): publish the completion AND force the 1ms receive timer to fire,
    // so both events are pending when the process next runs. The slice-start
    // gate must apply the completion, drop the timer mark as stale, and
    // clear receive_timeout so no later wait can arm a bogus timer at the
    // stale resume position.
    let fired = Arc::new(AtomicBool::new(false));
    let fired_by_hook = Arc::clone(&fired);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap != ParkGap::WaitStored
            || pid == shared.standard_io_pid
            || fired_by_hook.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let call_id = RACE_ID.load(Ordering::Acquire);
        assert!(
            shared.publish_suspension_result(
                pid,
                call_id,
                crate::scheduler::suspension::SuspensionResultPayload::host(Term::small_int(33))
                    .expect("immediate host payload"),
            )
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        timer_integration::tick_timers(shared);
    }));

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert!(fired.load(Ordering::Acquire), "park gap was exercised");
    assert_eq!(exit_value(&scheduler, pid), Some(Term::small_int(33)));
    assert_eq!(
        RACE_RUNS.load(Ordering::Acquire),
        1,
        "the timeout re-executed an await whose completion already arrived"
    );
    scheduler.shutdown();
}

// --- Reviewer fix F1: publish_suspension_result resolves concurrent
// publishers newest-id-wins inside the result-slot lock, so a publisher
// that passed the pre-check and then stalled across a timeout re-entry can
// never clobber the fresher completion published meanwhile. ---

#[test]
fn stale_publisher_cannot_clobber_a_fresher_completion() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = single_thread_scheduler(&registry);
    let shared = &scheduler.shared;
    let host = |value: i64| {
        crate::scheduler::suspension::SuspensionResultPayload::host(Term::small_int(value))
            .expect("immediate host payload")
    };
    let pid = 4246;
    shared.process_table.spawn_with_pid(pid);

    // Newer replaces older: a stale unconsumed id-1 entry is overwritten by
    // the re-suspension's id-2 completion.
    shared.register_suspension_mirror(pid, 1, crate::process::SuspensionKind::HostAwait, false);
    assert!(shared.publish_suspension_result(pid, 1, host(1)));
    shared.register_suspension_mirror(pid, 2, crate::process::SuspensionKind::HostAwait, false);
    assert!(
        shared.publish_suspension_result(pid, 2, host(2)),
        "the fresher completion must replace the stale entry"
    );
    assert_eq!(
        shared
            .suspension_results
            .get(&pid)
            .map(|entry| entry.call_id),
        Some(2)
    );

    // Older never replaces newer. This models publisher A passing the id-1
    // pre-check, stalling, and inserting only after the await timed out,
    // re-suspended as id 2, and publisher B's id-2 completion landed (the
    // doc-comment interleaving). The stalled pre-check is pinned by
    // re-registering the id-1 mirror; the resolution inside the result-slot
    // lock must still refuse the older id and keep B's entry.
    shared.register_suspension_mirror(pid, 1, crate::process::SuspensionKind::HostAwait, false);
    assert!(
        !shared.publish_suspension_result(pid, 1, host(1)),
        "a stale publisher must report failure instead of clobbering"
    );
    assert_eq!(
        shared
            .suspension_results
            .get(&pid)
            .map(|entry| entry.call_id),
        Some(2),
        "the fresher completion must survive the stale publisher"
    );
    scheduler.shutdown();
}

// --- Reviewer fix F2: a hook returning Suspend on the slice that just
// parked a result-gated await must be ignored — installing the Hook record
// would invalidate the await's call id, drop its completion, and re-execute
// the parked native on the eventual resume. ---

static HOOK_STOMP_RUNS: AtomicUsize = AtomicUsize::new(0);
static HOOK_STOMP_ID: AtomicU64 = AtomicU64::new(0);

fn hook_stomp_await_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    HOOK_STOMP_RUNS.fetch_add(1, Ordering::AcqRel);
    let call_id = context
        .request_await_suspend(None)
        .expect("attached process allocates a call id");
    HOOK_STOMP_ID.store(call_id, Ordering::Release);
    Ok(Term::NIL)
}

#[test]
fn hook_suspend_does_not_stomp_an_await_parked_slice() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("hook_stomp_await");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(&registry, module_name, hook_stomp_await_native, None);
    let scheduler = single_thread_scheduler(&registry);

    // Return Suspend exactly on the gated await's parking slice. The native
    // stores its call id before returning, and on the single scheduler
    // thread the hook runs synchronously at the end of that same slice, so
    // the first hook invocation observing the id is the await's own slice.
    let hook_suspended = Arc::new(AtomicBool::new(false));
    let hook_latch = Arc::clone(&hook_suspended);
    scheduler.hook().register(move |_event| {
        if HOOK_STOMP_ID.load(Ordering::Acquire) != 0 && !hook_latch.swap(true, Ordering::AcqRel) {
            HookDecision::Suspend
        } else {
            HookDecision::Continue
        }
    });

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || hook_suspended.load(Ordering::Acquire));
    wait_until(10_000, || {
        lock_or_recover(&scheduler.shared.wait_set)
            .waiting
            .contains_key(&pid)
    });
    assert_eq!(
        scheduler
            .shared
            .suspensions
            .get(&pid)
            .map(|mirror| mirror.kind),
        Some(crate::process::SuspensionKind::HostAwait),
        "the hook suspend stomped the await's suspension record"
    );

    // The await's completion still applies, and the native never re-runs.
    let call_id = HOOK_STOMP_ID.load(Ordering::Acquire);
    assert!(
        scheduler.wake_with_result_for(pid, call_id, Term::small_int(77)),
        "the await's published completion must still be accepted"
    );
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert_eq!(exit_value(&scheduler, pid), Some(Term::small_int(77)));
    assert_eq!(
        HOOK_STOMP_RUNS.load(Ordering::Acquire),
        1,
        "the hook stomp re-executed the parked await native"
    );
    scheduler.shutdown();
}

// --- Defect 6 (B-5b): a dirty native can re-suspend as a host await and can
// trampoline a closure — its follow-up requests are honored instead of being
// discarded by the bridge. ---

static DIRTY_RESUSPEND_RUNS: AtomicUsize = AtomicUsize::new(0);
static DIRTY_RESUSPEND_PARKED: AtomicBool = AtomicBool::new(false);

fn dirty_resuspend_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    DIRTY_RESUSPEND_RUNS.fetch_add(1, Ordering::AcqRel);
    // Detached context: the call id is allocated later, on the owning
    // thread, when the suspend request is applied.
    assert_eq!(context.request_await_suspend(None), None);
    DIRTY_RESUSPEND_PARKED.store(true, Ordering::Release);
    Ok(Term::NIL)
}

#[test]
fn dirty_native_can_resuspend_as_a_gated_host_await() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("dirty_resuspend");
    let registry = Arc::new(ModuleRegistry::new());
    let module = native_call_module(
        &registry,
        module_name,
        dirty_resuspend_native,
        Some(crate::scheduler::DirtySchedulerKind::Cpu),
    );
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || DIRTY_RESUSPEND_PARKED.load(Ordering::Acquire));
    // Wait for the re-suspension to become current: a HostAwait mirror
    // replaces the DirtyCall mirror once the owning thread applies the
    // dirty native's suspend request.
    wait_until(10_000, || {
        scheduler
            .shared
            .suspensions
            .get(&pid)
            .is_some_and(|mirror| mirror.kind == crate::process::SuspensionKind::HostAwait)
    });
    // A message must not wake the gated re-suspension (the dirty call
    // instruction would re-submit the dirty native).
    assert!(scheduler.enqueue_atom_message(pid, Atom::OK));
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(DIRTY_RESUSPEND_RUNS.load(Ordering::Acquire), 1);
    assert!(!scheduler.shared.exit_tombstones.contains_key(&pid));

    // The pid-resolved completion resumes it with the awaited value.
    assert!(scheduler.wake_with_result(pid, Term::small_int(55)));
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert_eq!(exit_value(&scheduler, pid), Some(Term::small_int(55)));
    assert_eq!(
        DIRTY_RESUSPEND_RUNS.load(Ordering::Acquire),
        1,
        "the dirty native was re-submitted"
    );
    scheduler.shutdown();
}

fn dirty_trampoline_resume(
    _state: crate::native::AionTimeoutContinuation,
    closure_result: Term,
    _context: &mut crate::native::ProcessContext<'_>,
) -> Result<crate::native::stdlib_stubs::maps_bifs::ContinuationStep, Term> {
    Ok(crate::native::stdlib_stubs::maps_bifs::ContinuationStep::Done(closure_result))
}

fn dirty_trampoline_native(
    args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    let fun = args.first().copied().expect("closure argument");
    context.set_continuation_trampoline(
        fun,
        vec![],
        crate::native::NativeContinuation::AionTimeout(crate::native::AionTimeoutContinuation {
            state_id: 1,
            resume: dirty_trampoline_resume,
        }),
    );
    Ok(Term::NIL)
}

#[test]
fn dirty_native_can_trampoline_a_closure() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("dirty_trampoline");
    let callback_atom = atoms.intern("dirty_cb@anon");
    let callback_id = crate::loader::lambda_unique_id(&atoms, module_name, callback_atom, 0, 0)
        .expect("callback id");
    let registry = Arc::new(ModuleRegistry::new());
    let mut module = test_module(
        module_name,
        vec![
            // Build the closure for lambda 0 into x0, hand it to the dirty
            // native, return its (trampolined) result.
            Instruction::MakeFun {
                operands: vec![Operand::Unsigned(0)],
            },
            Instruction::CallExt {
                arity: Operand::Unsigned(1),
                import: Operand::Unsigned(0),
            },
            Instruction::Return,
            // Closure body: return 88.
            Instruction::Label { label: 5 },
            Instruction::Move {
                source: Operand::Unsigned(88),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
    );
    module.lambdas.push(crate::loader::LambdaEntry {
        function: callback_atom,
        arity: 0,
        label: 5,
        num_free: 0,
        unique_id: callback_id,
    });
    module.resolved_imports.push(crate::module::ResolvedImport {
        module: module_name,
        function: module_name,
        arity: 1,
        target: crate::module::ResolvedImportTarget::Native(crate::native::NativeEntry {
            function: dirty_trampoline_native,
            dirty_kind: Some(crate::scheduler::DirtySchedulerKind::Cpu),
            capability: Capability::Pure,
        }),
    });
    let module = registry.insert(module);
    let scheduler = single_thread_scheduler(&registry);

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert_eq!(
        exit_value(&scheduler, pid),
        Some(Term::small_int(88)),
        "the dirty native's trampolined closure result is the call's result"
    );
    scheduler.shutdown();
}
