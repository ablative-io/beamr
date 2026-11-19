//! Running processes: slices to completion, entry at the exported function,
//! resuming a yielded process on its pinned module, and fair rescheduling.

use super::*;

#[test]
fn single_process_runs_to_completion_and_is_removed() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("simple");
    let registry = Arc::new(ModuleRegistry::new());
    let module = test_module(module_name, vec![Instruction::Return]);
    let module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };

    wait_until(2_000, || scheduler.process_table().get(pid).is_none());
    scheduler.shutdown();
}

#[test]
fn exported_spawn_starts_at_entry_function_with_args() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("entry_mod");
    let function = atoms.intern("main");
    let mut module = test_module(
        module_name,
        vec![
            Instruction::Label { label: 7 },
            Instruction::Move {
                source: Operand::X(0),
                destination: Operand::X(1),
            },
            Instruction::Return,
        ],
    );
    module.exports.insert((function, 1), 7);
    let registry = Arc::new(ModuleRegistry::new());
    registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let pid = scheduler
        .spawn(
            module_name,
            function,
            vec![Term::try_small_int(42).unwrap_or(Term::NIL)],
        )
        .unwrap_or_else(|error| panic!("spawn succeeds: {error}"));

    wait_until(2_000, || scheduler.process_table().get(pid).is_none());
    scheduler.shutdown();
}

#[test]
fn execute_slice_resumes_yielded_process_with_pinned_module_version() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("slice_pin");
    let registry = Arc::new(ModuleRegistry::new());
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
    let module_v1 = registry.insert(test_module(
        module_name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::CallOnly {
                arity: Operand::Unsigned(0),
                label: Operand::Label(1),
            },
        ],
    ));
    let (dirty_completion_shutdown_tx, dirty_completion_shutdown_rx) =
        crossbeam_channel::bounded::<()>(0);
    let shared = Arc::new(SharedState {
        shutdown: AtomicBool::new(false),
        failure: super::failure::FailureLatch::new(1),
        worker_exits: super::failure::WorkerExits::new(1),
        shutdown_skipped: Mutex::new(Vec::new()),
        process_table: ProcessTable::new(),
        module_registry: Arc::clone(&registry),
        namespace_store: {
            let store = DashMap::new();
            store.insert(NamespaceId::DEFAULT, Arc::clone(&registry));
            store
        },
        next_namespace_id: AtomicU64::new(1),
        spawn_counter: AtomicUsize::new(0),
        thread_count: 1,
        dirty_cpu: super::service::ServiceMode::Owned(dirty::DirtyPool::with_queue_depth(
            "dirty-test-cpu",
            1,
            1,
        )),
        dirty_io: super::service::ServiceMode::Owned(dirty::DirtyPool::with_queue_depth(
            "dirty-test-io",
            1,
            1,
        )),
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
    let mut process = Process::new(1, DEFAULT_HEAP_SIZE);
    process.set_code_position(Some(CodePosition {
        module: module_name,
        instruction_pointer: 0,
    }));
    process.set_current_module(Arc::clone(&module_v1));

    let _module_v2 = registry.insert(test_module(module_name, vec![Instruction::Return]));

    let SliceOutcome::Requeue(resumed) = execute_slice(&shared, &mut process) else {
        panic!("pinned loop should yield again instead of using reloaded return-only module");
    };
    assert!(
        resumed
            .current_module()
            .is_some_and(|current| Arc::ptr_eq(current, &module_v1))
    );
}

#[test]
fn yielded_process_is_rescheduled() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("loopy");
    let registry = Arc::new(ModuleRegistry::new());
    let module = test_module(
        module_name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::CallOnly {
                arity: Operand::Unsigned(0),
                label: Operand::Label(1),
            },
        ],
    );
    let module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let pid = match scheduler.spawn_process(&module) {
        Ok(pid) => pid,
        Err(error) => panic!("scaffold spawn refused: {error}"),
    };
    std::thread::sleep(std::time::Duration::from_millis(75));

    assert!(scheduler.process_table().get(pid).is_some());
    scheduler.shutdown();
}

#[test]
fn multiple_processes_fairly_complete() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("multi");
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(test_module(module_name, vec![Instruction::Return]));
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(2),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let pids: Vec<_> = (0..20)
        .map(|_| match scheduler.spawn_process(&module) {
            Ok(pid) => pid,
            Err(error) => panic!("scaffold spawn refused: {error}"),
        })
        .collect();

    wait_until(3_000, || {
        pids.iter()
            .all(|pid| scheduler.process_table().get(*pid).is_none())
    });
    scheduler.shutdown();
}
