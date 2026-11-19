//! Constructing a scheduler: worker count and names, node identity, replay
//! mode, distribution config, and the counts it reports.

use super::*;

#[test]
fn replay_scheduler_forces_single_worker_even_with_thread_override() {
    let scheduler = Scheduler::new_replay(
        SchedulerConfig {
            thread_count: Some(3),
            ..SchedulerConfig::default()
        },
        ReplayLog::default(),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("replay scheduler starts: {error}"));

    assert_eq!(scheduler.thread_count(), 1);
    scheduler.shutdown();
}

#[test]
fn replay_driver_exposes_recorded_schedule_order_without_run_queue_pop() {
    let scheduler = Scheduler::new_replay(
        SchedulerConfig::default(),
        ReplayLog::new(vec![
            ReplayEvent::Schedule(RecordedSchedule {
                pid: 3,
                scheduler_index: 0,
                reduction_budget: 11,
                reductions_consumed: 5,
            }),
            ReplayEvent::Schedule(RecordedSchedule {
                pid: 1,
                scheduler_index: 0,
                reduction_budget: 7,
                reductions_consumed: 2,
            }),
        ]),
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("replay scheduler starts: {error}"));

    let driver = scheduler
        .shared
        .replay_driver
        .as_ref()
        .expect("replay driver installed");
    let mut guard = driver.lock().expect("replay driver lock");
    assert_eq!(guard.peek_schedule().map(|event| event.pid), Some(3));
    assert_eq!(
        guard
            .next_schedule(0)
            .unwrap_or_else(|error| panic!("first schedule: {error}"))
            .pid,
        3
    );
    assert_eq!(guard.peek_schedule().map(|event| event.pid), Some(1));
    assert_eq!(
        guard
            .next_schedule(0)
            .unwrap_or_else(|error| panic!("second schedule: {error}"))
            .pid,
        1
    );
    assert!(guard.is_complete());
    drop(guard);
    scheduler.shutdown();
}

fn block_on_ready(future: ResolveFuture<'_>) -> Result<std::net::SocketAddr, ResolveError> {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = future;
    match future.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("resolver test future should be ready immediately"),
    }
}

#[test]
fn default_distribution_config_resolves_nothing() {
    assert!(SchedulerConfig::default().distribution.is_none());
    assert_eq!(SchedulerConfig::default().jit_threshold, None);

    // Honest None (spec §3.6): the default profile builds NO distribution, so
    // the config accessor is absent rather than exposing a default resolver.
    let default_scheduler = Scheduler::new(
        SchedulerConfig::default(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("scheduler should start");
    assert_eq!(
        default_scheduler.jit_profiler().current_threshold(),
        crate::jit::DEFAULT_JIT_THRESHOLD
    );
    assert!(
        default_scheduler.try_distribution_config().is_none(),
        "distribution: None ⇒ no config to read"
    );
    default_scheduler.shutdown();

    // The empty-resolver behavior this test implicitly pinned now lives on the
    // explicit full-runtime profile it was assuming (spec §6): full_runtime()
    // turns distribution on with a default config, exactly the state this pin
    // needs.
    let dist_scheduler = Scheduler::with_services(
        SchedulerConfig::default(),
        SchedulerServices::full_runtime(),
        Arc::new(ModuleRegistry::new()),
        NativeBifs::none(),
    )
    .expect("scheduler should start");
    assert_eq!(
        block_on_ready(
            dist_scheduler
                .try_distribution_config()
                .expect("distribution owned")
                .resolver
                .resolve("missing@localhost")
        ),
        Err(ResolveError::NotFound)
    );
    dist_scheduler.shutdown();
}

#[test]
fn scheduler_creates_requested_thread_count_and_names() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(4),
            ..SchedulerConfig::default()
        },
        registry,
        NativeBifs::none(),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    assert_eq!(scheduler.thread_count(), 4);
    assert_eq!(
        scheduler
            .try_dirty_cpu_pool()
            .expect("default profile owns a dirty CPU pool")
            .thread_count(),
        num_cpus::get()
    );
    assert_eq!(
        scheduler
            .try_dirty_io_pool()
            .expect("default profile owns a dirty IO pool")
            .thread_count(),
        dirty::DEFAULT_DIRTY_IO_THREADS
    );
    assert_eq!(
        scheduler.worker_names(),
        &[
            "beamr-sched-0",
            "beamr-sched-1",
            "beamr-sched-2",
            "beamr-sched-3"
        ]
    );

    scheduler.shutdown();
}

#[test]
fn scheduler_defaults_to_nonode_nohost_local_node() {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let scheduler = Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        Arc::clone(&atom_table),
        Arc::new(BifRegistryImpl::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let local_node = scheduler.local_node();
    assert_eq!(atom_table.resolve(local_node.name), Some("nonode@nohost"));
    assert_eq!(local_node.creation, 0);
    assert!(local_node.is_local(&scheduler.shared.local_node));

    scheduler.shutdown();
}

#[test]
fn scheduler_uses_configured_local_node_identity() {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let scheduler = Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            node_name: Some("worker@example.test".to_string()),
            creation: Some(7),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
        Arc::clone(&atom_table),
        Arc::new(BifRegistryImpl::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let local_node = scheduler.local_node();
    assert_eq!(
        atom_table.resolve(local_node.name),
        Some("worker@example.test")
    );
    assert_eq!(local_node.creation, 7);

    scheduler.shutdown();
}

#[test]
fn shared_state_metric_accessors_report_scheduler_process_and_atom_counts() {
    let atom_table = Arc::new(AtomTable::with_common_atoms());
    let extra_atom = atom_table.intern("scheduler_metrics_extra");
    assert!(atom_table.resolve(extra_atom).is_some());
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(2),
            ..SchedulerConfig::default()
        },
        registry,
        Arc::clone(&atom_table),
        Arc::new(BifRegistryImpl::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    assert_eq!(scheduler.scheduler_count(), 2);
    assert_eq!(scheduler.thread_count(), scheduler.scheduler_count());
    // Standard IO server is pre-registered as process 0.
    assert_eq!(scheduler.process_count(), 1);
    assert_eq!(scheduler.atom_count(), atom_table.len());
    assert_eq!(scheduler.atom_limit(), atom_table.limit());

    let pid = scheduler.shared.next_pid.fetch_add(1, Ordering::Relaxed);
    scheduler.process_table().spawn_with_pid(pid);
    assert_eq!(scheduler.process_count(), 2);
    let removed = scheduler.process_table().remove(pid);
    assert!(removed.is_some());
    assert_eq!(scheduler.process_count(), 1);

    scheduler.shutdown();
}

#[test]
fn scheduler_default_thread_count_matches_available_parallelism() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(SchedulerConfig::default(), registry, NativeBifs::none())
        .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let expected = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    assert_eq!(scheduler.thread_count(), expected);

    scheduler.shutdown();
}
