//! Wakeups: event-driven delivery wakes a waiting process, a completion for a
//! dead pid does not orphan state, and idle workers park.

use super::*;

#[test]
fn tombstone_after_wait_store_prevents_wait_parking() {
    let atom_table = Arc::new(crate::atom::AtomTable::new());
    // One owned distribution bundle, no outbound sender (spec §3.6).
    let distribution = super::service::ServiceMode::Owned(
        super::distribution_service::DistributionService::build(
            DistributionConfig::default(),
            Arc::clone(&atom_table),
            "local@test",
            0,
            false,
        ),
    );
    let (dirty_completion_shutdown_tx, dirty_completion_shutdown_rx) =
        crossbeam_channel::bounded::<()>(0);
    let shared = Arc::new(SharedState {
        shutdown: AtomicBool::new(false),
        failure: super::failure::FailureLatch::new(1),
        worker_exits: super::failure::WorkerExits::new(1),
        shutdown_skipped: Mutex::new(Vec::new()),
        process_table: ProcessTable::new(),
        module_registry: Arc::new(ModuleRegistry::new()),
        namespace_store: {
            let registry = Arc::new(ModuleRegistry::new());
            let store = DashMap::new();
            store.insert(NamespaceId::DEFAULT, registry);
            store
        },
        next_namespace_id: AtomicU64::new(1),
        spawn_counter: AtomicUsize::new(0),
        thread_count: 1,
        next_pid: AtomicU64::new(0),
        wait_set: Mutex::new(WaitSet::default()),
        wake_condvar: Condvar::new(),
        idle_wake: super::execution::IdleWake::default(),
        process_bodies: DashMap::new(),
        exit_tombstones: exit_tombstones::BoundedTombstones::new(),
        exit_completion_watches: Arc::new(exit_events::ExitCompletionRegistry::new()),
        exit_results: DashMap::new(),
        exit_errors: DashMap::new(),
        exit_exceptions: DashMap::new(),
        suspensions: DashMap::new(),
        suspension_results: DashMap::new(),
        pending_resumes: DashMap::new(),
        link_set: Mutex::new(LinkSet::new()),
        monitor_set: Mutex::new(MonitorSet::new()),
        hook: Hook::new(),
        distribution,
        process_registry: DashMap::new(),
        timers: Arc::new(Mutex::new(TimerWheel::new())),
        expired_receive_timers: DashMap::new(),
        output_sink: Mutex::new(Arc::new(NullSink)),
        io_ring: super::service::ServiceMode::Disabled,
        io_registry: None,
        io_bridge: Mutex::new(None),
        io_facility: None,
        atom_table: Arc::clone(&atom_table),
        ets_registry: Arc::new(crate::ets::EtsRegistry::new(atom_table)),
        pg_registry: Arc::new(crate::distribution::pg::PgRegistry::new(
            &crate::atom::AtomTable::with_common_atoms(),
        )),
        bif_registry: Arc::new(crate::native::BifRegistryImpl::new()),
        capability_policy: Arc::new(crate::native::AllCapabilitiesPolicy),
        idle_parks: AtomicUsize::new(0),
        park_seams: super::execution::ParkSeams::new(1),
        park_hooks: super::execution::ParkHooks::default(),
        suspension_mirror_registrations: AtomicU64::new(0),
        dirty_suspension_allocations: AtomicU64::new(0),
        park_gap_hook: Mutex::new(None),
        mailbox_admission_hook: std::sync::OnceLock::new(),
        terminal_admission_hook: std::sync::OnceLock::new(),
        terminal_spawn_visits: AtomicUsize::new(0),
        terminal_body_visits: AtomicUsize::new(0),
        spawn_admission_hook: std::sync::OnceLock::new(),
        teardown_wait_hook: std::sync::OnceLock::new(),
        shutdown_dispatch_hook: std::sync::OnceLock::new(),
        dirty_cpu: super::service::ServiceMode::Owned(crate::scheduler::dirty::DirtyPool::new(
            "test-cpu", 1,
        )),
        dirty_io: super::service::ServiceMode::Owned(crate::scheduler::dirty::DirtyPool::new(
            "test-io", 1,
        )),
        file_io_ring: super::service::ServiceMode::Disabled,
        file_io_pending: DashMap::new(),
        file_io_orphans: DashMap::new(),
        file_io_results: DashMap::new(),
        file_io_canceled: DashSet::new(),
        standard_io_pid: u64::MAX,
        #[cfg(feature = "readiness")]
        readiness: super::service::ServiceMode::Disabled,
        #[cfg(feature = "readiness")]
        readiness_consumer: None,
        service_instances: super::inventory::ServiceInstances::mint(false),
        dirty_completion_spawns: AtomicU64::new(0),
        dirty_completions: Mutex::new(super::TeardownAdmissionRegistry::default()),
        dirty_completions_changed: Condvar::new(),
        dirty_completion_shutdown_tx: Mutex::new(Some(dirty_completion_shutdown_tx)),
        dirty_completion_shutdown_rx,
        standard_io: super::service::ServiceMode::Disabled,
        local_node: crate::distribution::Node::new(crate::atom::Atom::new(0), 0),
        jit_profiler: Arc::new(crate::jit::JitProfiler::new(1000)),
        jit_compiler: Arc::new(
            crate::jit::JitCompiler::new(crate::jit::JitSettings)
                .expect("host JIT compiler should initialize"),
        ),
        jit_cache: Arc::new(crate::jit::JitCache::new()),
        jit_enabled: AtomicBool::new(true),
        replay_driver: None,
        replay_mode: false,
        nif_private_data: None,
        #[cfg(feature = "telemetry")]
        telemetry_metrics: TelemetryMetricState::new(std::time::Duration::from_millis(100)),
    });
    let pid = 1;
    shared.process_table.spawn_with_pid(pid);
    let process = Process::new(pid, DEFAULT_HEAP_SIZE);
    shared.process_bodies.insert(
        pid,
        Mutex::new(ProcessSlot::Executing(ProcessMetadata {
            namespace_id: NamespaceId::DEFAULT,
            capabilities: process.capabilities().clone(),
            relations: crate::process::SharedRelations::default(),
            trap_exit: false,
            priority: process.priority(),
            current_mfa: None,
            heap_size: 0,
            binary_heap_size: 0,
            message_queue_len: 0,
            group_leader: process.group_leader(),
            logical_clock: process.logical_clock(),
            pending_exit_messages: Vec::new(),
            pending_down_messages: Vec::new(),
            pending_io_messages: Vec::new(),
            pending_distribution_payloads: Vec::new(),
            pending_local_messages: Vec::new(),
            pending_ets_transfer_messages: Vec::new(),
            pending_udp_messages: Vec::new(),
            pending_tcp_messages: Vec::new(),
        })),
    );
    shared.exit_tombstones.insert(pid, ExitReason::Error);

    store_runnable_process(&shared, process);
    assert!(cleanup_if_tombstoned_after_store(&shared, pid));

    let ws = lock_or_recover(&shared.wait_set);
    assert!(
        !ws.waiting.contains_key(&pid),
        "tombstoned process must not be parked after store-back"
    );
}

#[test]
fn mailbox_send_wakes_waiting_process_event_driven() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        registry,
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let pid = 42;
    scheduler.shared.process_table.spawn_with_pid(pid);
    // Park the pid under a scheduler index NO live worker owns. `wake_process`
    // preserves the stored index when it moves the pid `waiting -> woken`, and
    // `drain_woken` only consumes `woken` entries whose index matches the
    // draining worker. Pinning to an unowned index leaves the woken transition
    // observable from this test thread instead of racing the worker's drain — a
    // race that otherwise flakes this assertion under a loaded parallel suite.
    let unowned_scheduler_index = scheduler.thread_count();
    {
        let mut wait_set = lock_or_recover(&scheduler.shared.wait_set);
        wait_set.waiting.insert(pid, unowned_scheduler_index);
    }
    let mailbox = Mailbox::new();
    let sender = mailbox
        .sender()
        .with_wake_notifier(scheduler.wake_notifier(pid));
    let mut receiver_heap = Heap::new(16);

    sender
        .send(Term::small_int(7), &mut receiver_heap)
        .unwrap_or_else(|error| panic!("send succeeds: {error}"));

    let wait_set = lock_or_recover(&scheduler.shared.wait_set);
    assert!(!wait_set.waiting.contains_key(&pid));
    assert!(
        wait_set
            .woken
            .iter()
            .any(|(woken_pid, _)| *woken_pid == pid)
    );
    drop(wait_set);
    scheduler.shutdown();
}

#[test]
fn fired_timer_mark_for_a_dead_pid_does_not_orphan() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        registry,
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let shared = &scheduler.shared;

    // A pid absent from the process table models the timer thread losing
    // the race in `expire_timers`: it passed the liveness check while the
    // process was alive, then exit cleanup purged the table and the pid's
    // marks before the insert. Pids are never reused, so without the
    // post-insert double-check the mark would orphan forever.
    let dead_pid = 4242;
    timer_integration::mark_fired_receive_timer(shared, dead_pid, 7);
    assert!(
        shared.expired_receive_timers.get(&dead_pid).is_none(),
        "mark for a dead pid must be removed by the double-check"
    );

    // A live pid keeps its mark for the owning thread to consume.
    let live_pid = 4243;
    shared.process_table.spawn_with_pid(live_pid);
    timer_integration::mark_fired_receive_timer(shared, live_pid, 8);
    assert_eq!(
        shared
            .expired_receive_timers
            .get(&live_pid)
            .map(|marks| marks.clone()),
        Some(vec![8]),
        "mark for a live pid must survive"
    );
    scheduler.shutdown();
}

#[test]
fn file_io_result_for_a_dead_pid_does_not_orphan() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        registry,
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let shared = &scheduler.shared;

    fn file_completion(op_id: u64) -> crate::native::FileIoCompletion {
        crate::native::FileIoCompletion {
            op_id,
            continuation: crate::native::FileIoContinuation::Open,
            completion: crate::io::ring::IoCompletion {
                op_id,
                result: Err(std::io::Error::other("dead pid race")),
            },
        }
    }

    // A pid absent from the process table models the I/O drain losing the
    // race: it removed the pending entry while the process was alive, then
    // exit cleanup purged the table and the pid's results before the
    // insert. Pids are never reused, so without the post-insert
    // double-check the result would orphan forever.
    let dead_pid = 4244;
    execution::deliver_file_io_result(shared, dead_pid, file_completion(1));
    assert!(
        shared.file_io_results.get(&dead_pid).is_none(),
        "file-I/O result for a dead pid must be removed by the double-check"
    );

    // A live pid keeps its result for the owning thread to consume.
    let live_pid = 4245;
    shared.process_table.spawn_with_pid(live_pid);
    execution::deliver_file_io_result(shared, live_pid, file_completion(2));
    assert_eq!(
        shared
            .file_io_results
            .get(&live_pid)
            .map(|entry| entry.op_id),
        Some(2),
        "file-I/O result for a live pid must survive"
    );
    scheduler.shutdown();
}

#[test]
fn mailbox_send_does_not_wake_when_copy_fails() {
    let called = Arc::new(AtomicBool::new(false));
    let called_by_wake = Arc::clone(&called);
    let mailbox = Mailbox::new();
    let sender = mailbox.sender().with_wake_notifier(move || {
        called_by_wake.store(true, Ordering::Release);
    });
    let mut receiver_heap = Heap::new(0);
    let mut sender_words = [0_u64; 2];
    let too_large = boxed::write_cons(&mut sender_words, Term::small_int(1), Term::NIL)
        .unwrap_or_else(|| panic!("source cons fits"));

    assert!(sender.send(too_large, &mut receiver_heap).is_err());
    assert!(!called.load(Ordering::Acquire));
}

#[test]
fn idle_threads_park_instead_of_spinning() {
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

    wait_until(500, || scheduler.idle_park_count() > 0);
    scheduler.shutdown();
}
