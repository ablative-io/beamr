//! Shutdown: idempotence, dropping shared state, closing distribution
//! connections, and draining completion bridges.

use super::*;

#[test]
fn shutdown_is_idempotent() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(2),
            ..SchedulerConfig::default()
        },
        registry,
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    scheduler.shutdown();
    scheduler.shutdown();
}

/// FIX 2 regression: a non-replay scheduler's `SharedState` must actually drop
/// when the `Scheduler` drops. The distribution control-frame handler is stored
/// inside `SharedState`'s own `ConnectionManager`; if it captured a strong
/// `Arc<SharedState>` (as it did before the fix) the cycle `SharedState ->
/// distribution_connections -> control_frame_handler -> Arc<SharedState>` would
/// leak the scheduler forever. We hold ONLY a `Weak`, drop the scheduler, and
/// assert the `Weak` can no longer upgrade and reports a zero strong count.
///
/// This is a plain (non-async) `#[test]`, but `Scheduler::drop` runs the full
/// teardown — including the owned `DistSender` runtime drop — so it also exercises
/// FIX 1's dedicated-thread runtime drop reaching zero strong refs without panic.
#[test]
fn scheduler_shared_state_drops_without_leak() {
    let registry = Arc::new(ModuleRegistry::new());
    // Moved to the full-runtime profile (spec §6): the leak site is the
    // distribution control-frame handler, which only exists on a scheduler that
    // OWNS a distribution bundle. Since commit 4, `SchedulerConfig::default()`
    // builds no distribution, so `Scheduler::new` here would leave the cycle
    // site unbuilt and this pin could never fail. `full_runtime()` owns the
    // bundle, so the cycle site is genuinely exercised.
    let scheduler = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(2),
            ..SchedulerConfig::default()
        },
        SchedulerServices::full_runtime(),
        registry,
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    // A non-replay scheduler must have built the dist sender and registered the
    // control-frame handler (the leak site under test).
    assert!(
        !scheduler.shared.replay_mode,
        "test must run a non-replay scheduler (replay skips the control handler)"
    );
    assert!(
        scheduler.try_distribution_config().is_some(),
        "full_runtime() must own the distribution bundle whose control-frame \
         handler is the leak site under test"
    );

    // Downgrade to a Weak so this test holds NO strong reference of its own.
    let weak: std::sync::Weak<SharedState> = Arc::downgrade(&scheduler.shared);

    scheduler.shutdown();
    drop(scheduler);

    assert!(
        weak.upgrade().is_none(),
        "SharedState leaked: a strong Arc cycle outlived the Scheduler"
    );
    assert_eq!(
        weak.strong_count(),
        0,
        "SharedState strong count must reach zero after the Scheduler drops"
    );
}

#[test]
fn shutdown_closes_active_distribution_connections_before_returning() {
    // Spec §3.6 connection-complete shutdown: `Scheduler::shutdown()` must
    // close every active distribution connection — peer sees EOF, table
    // empties — not merely join the runtime workers. Regression pin for the
    // teardown that left retained write halves (and their sockets) alive
    // until the last `Arc<DistConnection>` dropped.
    use std::io::Read;

    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            distribution: Some(crate::distribution::DistributionConfig::default()),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let manager = scheduler.shared.distribution_connections_or_panic().clone();
    let node = scheduler.shared.atom_table.intern("peer@teardown");
    let (server, mut client, addr) = super::connection_lifecycle_tests::socket_pair();
    {
        let handle = scheduler.shared.dist_sender_or_panic().handle();
        let _context = handle.enter();
        manager
            .register_test_connection(node, addr, server)
            .unwrap_or_else(|error| panic!("register test connection: {error}"));
    }
    assert_eq!(manager.connected_nodes(), vec![node]);

    scheduler.shutdown();

    // Table emptied and Down delivered by the time shutdown returns (INV-SYNC
    // via disconnect_all); the peer's blocking read observes EOF (FIN from the
    // taken write half), bounded only as a hang guard.
    assert!(
        manager.connected_nodes().is_empty(),
        "shutdown must remove every active connection from the table"
    );
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap_or_else(|error| panic!("set read timeout: {error}"));
    let mut buffer = [0u8; 8];
    match client.read(&mut buffer) {
        Ok(0) => {}
        Ok(n) => panic!("peer expected EOF, read {n} bytes"),
        // A reset also proves the socket is torn down.
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        Err(error) => panic!("peer expected EOF, read failed: {error}"),
    }
}

static SHARED_DRAIN_ENTERED: AtomicUsize = AtomicUsize::new(0);
static SHARED_DRAIN_RELEASE: AtomicBool = AtomicBool::new(false);

/// 0 = unset; 1 = refused with SchedulerTearingDown; 2 = anything else.
static SHARED_DRAIN_SPAWN_OUTCOME: AtomicUsize = AtomicUsize::new(0);

fn shared_drain_blocking_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    SHARED_DRAIN_ENTERED.fetch_add(1, Ordering::AcqRel);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !SHARED_DRAIN_RELEASE.load(Ordering::Acquire) {
        assert!(
            std::time::Instant::now() < deadline,
            "shared-drain native never released"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // The owning scheduler has SHUT DOWN by the time the release fires (the
    // test orders it so): a mutation through the retained facilities must be
    // REFUSED with the teardown error, before MFA resolution.
    let outcome = context.spawn_facility().map(|facility| {
        facility.spawn(
            context.pid().unwrap_or(0),
            Atom::OK,
            Atom::OK,
            Vec::new(),
            None,
        )
    });
    let recorded = match outcome {
        Some(Err(crate::native::spawn::SpawnError::SchedulerTearingDown)) => 1,
        _ => 2,
    };
    SHARED_DRAIN_SPAWN_OUTCOME.store(recorded, Ordering::Release);
    Ok(Term::atom(Atom::OK))
}

fn shared_drain_quick_native(
    _args: &[Term],
    _context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    Ok(Term::atom(Atom::OK))
}

/// Round-1 major 1 (§4 step 3): a scheduler sharing an embedder-owned dirty
/// pool must JOIN its completion bridges at shutdown even while a shared-pool
/// job is still running — shutdown returns bounded, no bridge thread remains,
/// no completion is delivered into the dead scheduler, and the pool keeps
/// serving a co-resident scheduler. Verified failing (shutdown hung on the
/// blocked bridge / bridge outlived shutdown) with the drain reverted.
#[test]
fn shutdown_drains_completion_bridges_while_a_shared_pool_job_is_still_running() {
    // TWO workers: A's job blocks one; the second keeps serving B while A's
    // job is still parked, so the pool-stays-functional half asserts under the
    // strongest condition (the blocked job still occupying a worker).
    let pool = Arc::new(DirtyPool::with_queue_depth("shared-dirty-drain", 2, 8));
    let registry = Arc::new(ModuleRegistry::new());
    let atoms = AtomTable::new();
    let blocking_name = atoms.intern("shared_drain_blocking");
    let quick_name = atoms.intern("shared_drain_quick");
    let blocking_module = native_call_module(
        &registry,
        blocking_name,
        shared_drain_blocking_native,
        Some(DirtySchedulerKind::Cpu),
    );
    let quick_module = native_call_module(
        &registry,
        quick_name,
        shared_drain_quick_native,
        Some(DirtySchedulerKind::Cpu),
    );

    SHARED_DRAIN_ENTERED.store(0, Ordering::Release);
    SHARED_DRAIN_RELEASE.store(false, Ordering::Release);
    SHARED_DRAIN_SPAWN_OUTCOME.store(0, Ordering::Release);

    let scheduler_a = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal().shared_dirty_cpu(Arc::clone(&pool)),
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler A starts: {error}"));
    let scheduler_b = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal().shared_dirty_cpu(Arc::clone(&pool)),
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler B starts: {error}"));

    // A's dirty job enters the shared pool and BLOCKS there.
    let blocked_pid = match scheduler_a.spawn_process(&blocking_module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    let entered_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while SHARED_DRAIN_ENTERED.load(Ordering::Acquire) != 1 {
        if std::time::Instant::now() >= entered_deadline {
            panic!(
                "job never entered: tombstoned={} exit_reason={:?} exit_error={:?}",
                scheduler_a
                    .shared
                    .exit_tombstones
                    .contains_key(&blocked_pid),
                scheduler_a.peek_exit_reason(blocked_pid),
                scheduler_a.take_exit_error(blocked_pid),
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    // Shutdown must return while the job is STILL blocked: the drain wakes the
    // bridge (which exits without delivering) rather than waiting for the job.
    let weak_a = Arc::downgrade(&scheduler_a.shared);
    let started = std::time::Instant::now();
    scheduler_a.shutdown();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "shutdown must not wait for the shared-pool job"
    );
    assert_eq!(
        scheduler_a.dirty_completions_live_count(),
        0,
        "no completion bridge of the dead scheduler survives its shutdown"
    );
    assert!(
        !SHARED_DRAIN_RELEASE.load(Ordering::Acquire),
        "the job is still blocked — the bridge exited without it"
    );
    drop(scheduler_a);

    // The pool is untouched: B still submits and completes dirty work.
    let b_pid = match scheduler_b.spawn_process(&quick_module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    wait_until(10_000, || {
        scheduler_b.shared.exit_tombstones.contains_key(&b_pid)
    });
    assert_eq!(
        scheduler_b.peek_exit_reason(b_pid),
        Some(ExitReason::Normal)
    );

    // Release A's job. The worker's send lands on a dropped receiver and is
    // discarded; the native's post-release spawn attempt through its retained
    // facility must be REFUSED with the teardown error (round-2 major 3), and
    // A's process table must not grow.
    let shared_a = weak_a.upgrade().expect("job still retains A's state");
    let processes_before = shared_a.process_bodies.len();
    SHARED_DRAIN_RELEASE.store(true, Ordering::Release);
    wait_until(10_000, || {
        SHARED_DRAIN_SPAWN_OUTCOME.load(Ordering::Acquire) != 0
    });
    assert_eq!(
        SHARED_DRAIN_SPAWN_OUTCOME.load(Ordering::Acquire),
        1,
        "a post-shutdown spawn from the surviving dirty native is refused \
         with SchedulerTearingDown"
    );
    assert_eq!(
        shared_a.process_bodies.len(),
        processes_before,
        "the refused spawn created no process in the dead scheduler"
    );
    drop(shared_a);
    wait_until(10_000, || weak_a.upgrade().is_none());
    scheduler_b.shutdown();
}
