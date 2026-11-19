//! Disabled services refuse before any suspension: the dirty CPU pool, the
//! file ring and the standard-IO ring.

use super::*;

// ---------------------------------------------------------------------------
// Disabled dirty pools: refusal before any side effect (spec §3.2). The
// gated-suspension hazard is the scariest arm — a suspension registered
// against a pool with no worker parks the process FOREVER (readiness contract
// C2: no message can wake a gated await). These pin the refusal-first
// ordering, the typed service-unavailable surface, and that an unrelated
// process on the SAME scheduler keeps making progress.
// ---------------------------------------------------------------------------

static DISABLED_DIRTY_PROBE_RUNS: AtomicUsize = AtomicUsize::new(0);
static DISABLED_PEER_PROGRESS: AtomicUsize = AtomicUsize::new(0);

fn disabled_dirty_probe_native(
    _args: &[Term],
    _context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    // A refused dispatch never reaches this body: the counter proves the pool
    // was never submitted into.
    DISABLED_DIRTY_PROBE_RUNS.fetch_add(1, Ordering::AcqRel);
    Ok(Term::small_int(42))
}

fn disabled_peer_progress_native(
    _args: &[Term],
    _context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    DISABLED_PEER_PROGRESS.fetch_add(1, Ordering::AcqRel);
    Ok(Term::small_int(7))
}

#[test]
fn disabled_dirty_cpu_refuses_before_suspension_without_wedging_the_scheduler() {
    let atoms = AtomTable::new();
    let registry = Arc::new(ModuleRegistry::new());
    let dirty_name = atoms.intern("disabled_dirty_cpu");
    let peer_name = atoms.intern("disabled_dirty_peer");
    let dirty_module = native_call_module(
        &registry,
        dirty_name,
        disabled_dirty_probe_native,
        Some(DirtySchedulerKind::Cpu),
    );
    let peer_module = native_call_module(&registry, peer_name, disabled_peer_progress_native, None);

    // dirty-cpu explicitly requested OFF; dirty-io stays owned so only the CPU
    // pool is disabled.
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(0),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    // Assertion-2 instance: the disabled pool is a §5 Disabled entry — zero
    // threads, zero fds, DISABLED sentinel instance.
    let cpu_entry = scheduler
        .service_inventory()
        .into_iter()
        .find(|entry| entry.service == inventory::DIRTY_CPU)
        .expect("dirty-cpu inventory entry");
    assert_eq!(cpu_entry.mode, ServiceModeLabel::Disabled);
    assert_eq!(cpu_entry.actual, 0);
    assert_eq!(cpu_entry.configured, 0);
    assert!(cpu_entry.thread_names.is_empty());
    assert!(cpu_entry.fd_classes.is_empty());
    assert_eq!(cpu_entry.instance, ServiceInstanceId::DISABLED);

    DISABLED_DIRTY_PROBE_RUNS.store(0, Ordering::Release);
    DISABLED_PEER_PROGRESS.store(0, Ordering::Release);

    // Boundary snapshots for the refusal ORDERING claim: refusal must precede
    // the WHOLE gated-suspension sequence (call-id allocation ->
    // set_suspension -> mirror registration) and the dirty submit, so the
    // refused call moves none of these instruments. An end-state check alone
    // (no mirror left behind) would also pass a register-then-refuse-then-
    // clean-up implementation; the allocation counter pins the sequence's
    // first side effect and the mirror counter its last, so a check that
    // regresses to anywhere inside the sequence moves one of them.
    let allocations_before = scheduler.dirty_suspension_allocation_count();
    let mirrors_before = scheduler.suspension_mirror_registration_count();
    let completion_spawns_before = scheduler
        .shared
        .dirty_completion_spawns
        .load(Ordering::Acquire);

    let dirty_pid = match scheduler.spawn_process(&dirty_module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };

    // The refused process exits PROMPTLY — a park-forever bug would hang this
    // bounded wait instead of tombstoning.
    wait_until(5_000, || {
        scheduler.shared.exit_tombstones.contains_key(&dirty_pid)
    });
    assert_eq!(
        scheduler.peek_exit_reason(dirty_pid),
        Some(ExitReason::Error)
    );

    // Typed, distinguishable service-unavailable error (Q-B): NOT Badarg, and
    // it names the disabled service for the embedder.
    assert_eq!(
        scheduler.take_exit_error(dirty_pid),
        Some(crate::error::ExecError::ServiceUnavailable {
            service: inventory::DIRTY_CPU
        }),
    );

    // Park-forever hazard NEGATIVE: refusal preceded suspension registration,
    // so no gated suspension survives for the dead pid.
    assert!(
        !scheduler.shared.suspensions.contains_key(&dirty_pid),
        "a refused dirty call must leave no gated suspension behind"
    );

    // The ordering itself: the refused call never entered the suspension
    // sequence at all — no call id allocated, no mirror registered, no
    // completion thread spawned; not merely cleaned up afterwards. A refusal
    // regressing to anywhere inside the sequence moves a counter and fails.
    assert_eq!(
        scheduler.dirty_suspension_allocation_count(),
        allocations_before,
        "a refused dirty call must never allocate a suspension call id"
    );
    assert_eq!(
        scheduler.suspension_mirror_registration_count(),
        mirrors_before,
        "a refused dirty call must never register a suspension mirror"
    );
    assert_eq!(
        scheduler
            .shared
            .dirty_completion_spawns
            .load(Ordering::Acquire),
        completion_spawns_before,
        "a refused dirty call must never spawn a completion thread"
    );

    // The pool was never submitted into: the native body never ran.
    assert_eq!(DISABLED_DIRTY_PROBE_RUNS.load(Ordering::Acquire), 0);

    // An unrelated process on the SAME scheduler keeps making progress.
    let peer_pid = match scheduler.spawn_process(&peer_module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    let (peer_reason, _peer_result) = scheduler
        .run_until_exit(peer_pid)
        .unwrap_or_else(|failure| panic!("scheduler failed: {failure}"));
    assert_eq!(peer_reason, ExitReason::Normal);
    assert_eq!(DISABLED_PEER_PROGRESS.load(Ordering::Acquire), 1);

    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// Disabled file-IO ring: refusal before any suspension (spec §3.3). A Disabled
// file ring is absent from native services (no facility), so a file submit is
// refused at the `file_io_facility.is_none()` check — BEFORE
// `request_await_suspend` reaches the registrar. The file path suspends via the
// same host-await machinery the dirty path does, so the gated-suspension
// hazard is identical: a suspension registered against an absent ring would
// park the process forever. The instrument is the shared suspension-mirror
// counter; the positive control proves an ACCEPTED submit moves it.
// ---------------------------------------------------------------------------

/// Minimal [`FileIoFacility`](crate::native::FileIoFacility) for the positive
/// control: an accepted submit returns an op id and never touches a real ring.
struct GateFileFacility {
    ring: Arc<dyn crate::io::CompletionRing>,
}

impl crate::native::FileIoFacility for GateFileFacility {
    fn submit_file_io(
        &self,
        _pid: u64,
        _op: crate::io::IoOp,
        _continuation: crate::native::FileIoContinuation,
    ) -> u64 {
        7
    }

    fn track_submitted_file_io(
        &self,
        _pid: u64,
        _op_id: u64,
        _continuation: crate::native::FileIoContinuation,
    ) {
    }

    fn take_file_io_completion(&self, _pid: u64) -> Option<crate::native::FileIoCompletion> {
        None
    }

    fn cancel_pending_file_io_for_pid(&self, _pid: u64) {}

    fn ring(&self) -> &dyn crate::io::CompletionRing {
        self.ring.as_ref()
    }
}

#[test]
fn disabled_file_ring_refuses_file_submit_before_registering_a_suspension() {
    // A real scheduler gives us the live suspension-mirror instrument that the
    // file path shares with the dirty path.
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let shared = Arc::clone(&scheduler.shared);
    let registrar: Arc<dyn crate::native::SuspensionRegistrar> =
        Arc::new(super::suspension::SchedulerSuspensionRegistrar {
            shared: Arc::clone(&shared),
        });

    let pid = 500u64;
    let mut process = Process::new(pid, DEFAULT_HEAP_SIZE);
    let mut context = crate::native::ProcessContext::new();
    context.set_pid(Some(pid));
    context.attach_process(&mut process, 0);
    context.set_suspension_registrar(Some(Arc::clone(&registrar)));

    // NEGATIVE: no file facility — the Disabled file-ring shape (spec §3.3).
    // The submit refuses (badarg at the BIF surface, Q-B: existing atom; the
    // embedder-typed ServiceUnavailable half of Q-B lands with live-Disabled
    // construction in commit 5) and the refusal precedes suspension
    // registration, so the mirror counter is untouched and no gated
    // suspension is stranded for the pid.
    let mirrors_before = scheduler.suspension_mirror_registration_count();
    let refused = context.submit_file_io(
        crate::io::IoOp::Nop,
        crate::native::FileIoContinuation::Open,
    );
    assert!(
        refused.is_err(),
        "a Disabled file ring must refuse the submit"
    );
    assert_eq!(
        scheduler.suspension_mirror_registration_count(),
        mirrors_before,
        "a refused file submit must register no suspension mirror"
    );
    assert!(
        !shared.suspensions.contains_key(&pid),
        "a refused file submit must leave no gated suspension behind"
    );

    // POSITIVE control: with the facility present, the SAME submit registers
    // exactly one suspension mirror — a pin that cannot fail is not a pin.
    context.set_file_io_facility(Some(Arc::new(GateFileFacility {
        ring: Arc::from(crate::io::create_ring_with_prefix(
            crate::io::RingConfig::default(),
            "gate-file-mock",
        )),
    })));
    let accepted = context.submit_file_io(
        crate::io::IoOp::Nop,
        crate::native::FileIoContinuation::Open,
    );
    assert!(
        accepted.is_ok(),
        "a present file ring must accept the submit"
    );
    assert_eq!(
        scheduler.suspension_mirror_registration_count(),
        mirrors_before + 1,
        "an accepted file submit registers exactly one suspension mirror"
    );

    drop(context);
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// Disabled standard-IO ring (spec §3.4): no ring, no process 0. A replay
// scheduler is the one construction TODAY where the standard ring is Disabled
// (the composition API that disables it in a LIVE profile is commit 5). The
// standard ring is NEVER a live ring behind a disabled facade — a Disabled
// slot registers no process 0, so the server's completion poll loop can never
// hang a normal worker.
// ---------------------------------------------------------------------------

#[test]
fn disabled_standard_io_ring_registers_no_process_zero_and_reports_disabled() {
    let scheduler = Scheduler::new_replay(
        SchedulerConfig::default(),
        ReplayLog::default(),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("replay scheduler starts: {error}"));

    // No process 0: a live scheduler registers the standard-IO server as pid 0
    // (process_count == 1); a Disabled standard ring registers none.
    assert_eq!(
        scheduler.process_count(),
        0,
        "a Disabled standard-IO ring registers no process 0"
    );

    let by_service: StdHashMap<&'static str, inventory::ServiceInventoryEntry> = scheduler
        .service_inventory()
        .into_iter()
        .map(|entry| (entry.service, entry))
        .collect();

    // Standard ring: a §5 Disabled entry.
    let standard = &by_service[inventory::STANDARD_IO_RING];
    assert_eq!(standard.mode, ServiceModeLabel::Disabled);
    assert_eq!(standard.actual, 0);
    assert_eq!(standard.configured, 0);
    assert!(standard.thread_names.is_empty());
    assert_eq!(
        standard.instance,
        super::service::ServiceInstanceId::DISABLED
    );

    // File ring is likewise Disabled under replay (spec §3.3).
    let file = &by_service[inventory::FILE_IO_RING];
    assert_eq!(file.mode, ServiceModeLabel::Disabled);
    assert_eq!(file.actual, 0);
    assert_eq!(file.instance, super::service::ServiceInstanceId::DISABLED);

    scheduler.shutdown();
}
