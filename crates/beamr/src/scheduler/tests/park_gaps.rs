//! Deliveries and resumes that land in a park gap are not lost wakeups.

use super::*;

// ---------------------------------------------------------------------------
// Park-gap interleavings: deliveries and dirty resumes racing run_process's
// park sequences. The park_gap_hook runs synchronously at the exact gap, so
// each test pins one interleaving deterministically.
// ---------------------------------------------------------------------------

#[test]
fn delivery_in_the_wait_park_gap_is_not_a_lost_wakeup() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("wait_park_gap");
    let registry = Arc::new(ModuleRegistry::new());
    let module = receive_then_return_module(&registry, module_name);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    // Deliver + wake in the gap between the Wait arm's mailbox recheck and
    // its wait-set registration (interleaving 1 in the Wait arm comment):
    // the wake is a no-op because the pid is not registered yet, so only the
    // registered-before-recheck ordering schedules the process again.
    let delivered = Arc::new(AtomicBool::new(false));
    let delivered_by_hook = Arc::clone(&delivered);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap != ParkGap::WaitStored
            || pid == shared.standard_io_pid
            || delivered_by_hook.swap(true, Ordering::AcqRel)
        {
            return;
        }
        deliver_atom(shared, pid, Atom::OK);
        execution::wake_process(shared, pid);
    }));

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert!(delivered.load(Ordering::Acquire), "park gap was exercised");
    let exit_value = scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root());
    assert_eq!(exit_value, Some(Term::atom(Atom::OK)));
    scheduler.shutdown();
}

#[test]
fn delivery_after_wait_registration_schedules_the_process_once() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("wait_park_gap_late");
    let registry = Arc::new(ModuleRegistry::new());
    let module = receive_then_return_module(&registry, module_name);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    // Deliver + wake between registration and the mailbox recheck
    // (interleaving 2): the wake moves the pid to `woken`, and the recheck's
    // self-wake must back off so the process is scheduled exactly once.
    let delivered = Arc::new(AtomicBool::new(false));
    let delivered_by_hook = Arc::clone(&delivered);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap != ParkGap::WaitRegistered
            || pid == shared.standard_io_pid
            || delivered_by_hook.swap(true, Ordering::AcqRel)
        {
            return;
        }
        deliver_atom(shared, pid, Atom::OK);
        execution::wake_process(shared, pid);
    }));

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert!(delivered.load(Ordering::Acquire), "park gap was exercised");
    let exit_value = scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root());
    assert_eq!(exit_value, Some(Term::atom(Atom::OK)));
    scheduler.shutdown();
}

/// Gates the dirty native used by the suspend park-gap test so the real
/// completion bridge cannot publish a competing result before the test's
/// simulated bridge runs in the gap.
static SUSPEND_PARK_GAP_RELEASE: AtomicBool = AtomicBool::new(false);

fn suspend_park_gap_native(
    _args: &[Term],
    _context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    while !SUSPEND_PARK_GAP_RELEASE.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    Ok(Term::small_int(99))
}

fn suspend_park_gap_module(module_name: Atom) -> Module {
    let mut module = test_module(
        module_name,
        vec![
            Instruction::CallExt {
                arity: Operand::Unsigned(0),
                import: Operand::Unsigned(0),
            },
            Instruction::Return,
        ],
    );
    module.resolved_imports.push(crate::module::ResolvedImport {
        module: module_name,
        function: module_name,
        arity: 0,
        target: crate::module::ResolvedImportTarget::Native(crate::native::NativeEntry {
            function: suspend_park_gap_native,
            dirty_kind: Some(crate::scheduler::DirtySchedulerKind::Cpu),
            capability: Capability::Pure,
        }),
    });
    module
}

#[test]
fn dirty_resume_in_the_suspend_park_gap_is_not_lost() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("suspend_park_gap");
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(suspend_park_gap_module(module_name));
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    // Simulate the completion bridge landing in the gap between the
    // Suspended arm's store-back and its wait-set registration
    // (interleaving 2 in the Suspended arm comment): the resume flips the
    // status Suspended→Yielded but finds nothing in the wait set, so only
    // the arm's fallback unpark can schedule the process.
    let fired = Arc::new(AtomicBool::new(false));
    let fired_by_hook = Arc::clone(&fired);
    let resumed_in_gap = Arc::new(AtomicBool::new(false));
    let resumed_by_hook = Arc::clone(&resumed_in_gap);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap != ParkGap::SuspendStored
            || pid == shared.standard_io_pid
            || fired_by_hook.swap(true, Ordering::AcqRel)
        {
            return;
        }
        // Publish exactly the way the completion bridge does: under the
        // in-flight dirty call's id, read from the suspension mirror the
        // submission registered.
        let call_id = shared
            .suspensions
            .get(&pid)
            .map(|mirror| mirror.call_id)
            .expect("dirty suspension mirror registered before park");
        let published = shared.publish_suspension_result(
            pid,
            call_id,
            crate::scheduler::suspension::SuspensionResultPayload::Dirty(Box::new(
                crate::scheduler::dirty::DirtyResult {
                    result: Ok(Term::small_int(7)),
                    owned_result: None,
                    exception_class: crate::native::ExceptionClass::Error,
                    exception_stacktrace: Term::NIL,
                    suspend: None,
                    trampoline: None,
                },
            )),
        );
        assert!(published, "in-gap publish matches the current suspension");
        resumed_by_hook.store(
            timer_integration::resume_suspended(shared, pid),
            Ordering::Release,
        );
    }));

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut completed = false;
    while std::time::Instant::now() < deadline {
        if scheduler.shared.exit_tombstones.contains_key(&pid) {
            completed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Release the gated native before any assertion can panic: dropping the
    // scheduler joins the dirty pool, which would otherwise hang on the
    // still-spinning native.
    SUSPEND_PARK_GAP_RELEASE.store(true, Ordering::Release);
    assert!(fired.load(Ordering::Acquire), "park gap was exercised");
    assert!(
        !resumed_in_gap.load(Ordering::Acquire),
        "the in-gap resume must have failed the wait-set removal"
    );
    assert!(
        completed,
        "suspended process with a published dirty result was never resumed"
    );
    let exit_value = scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root());
    assert_eq!(exit_value, Some(Term::small_int(7)));
    scheduler.shutdown();
}
