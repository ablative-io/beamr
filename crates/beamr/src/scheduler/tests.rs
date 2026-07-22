use std::collections::HashMap as StdHashMap;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::task::{Context, Poll, Wake, Waker};

#[cfg(feature = "telemetry")]
use opentelemetry::Key;
#[cfg(feature = "telemetry")]
use opentelemetry::trace::{Span, TraceContextExt, Tracer};
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

use dashmap::{DashMap, DashSet};

use super::*;
use crate::atom::{Atom, AtomTable};
use crate::distribution::{ResolveError, ResolveFuture};
use crate::ets::{EtsTableMetadata, EtsTableType, Protection};
use crate::hook::{Hook, HookDecision};
use crate::io::NullSink;
use crate::loader::Instruction;
use crate::loader::decode::compact::Operand;
use crate::mailbox::Mailbox;
use crate::module::{Module, ModuleOrigin};
use crate::namespace::NamespaceId;
use crate::native::{Capability, CapabilitySet, SpawnFacility, SpawnOptions};
use crate::process::heap::{DEFAULT_HEAP_SIZE, Heap};
use crate::process::registry::ProcessTable;
use crate::process::{CodePosition, ExitReason, Priority};
use crate::replay::{RecordedSchedule, ReplayEvent, ReplayLog};
use crate::scheduler::execution::{
    SliceOutcome, cleanup_exited_process, cleanup_if_tombstoned_after_store, execute_slice,
    store_runnable_process, take_runnable_process,
};
use crate::supervision::link::LinkSet;
use crate::supervision::monitor::MonitorSet;
use crate::term::{Term, boxed};
use crate::timer::TimerWheel;

fn ets_metadata(name: Option<Atom>, owner: u64) -> EtsTableMetadata {
    EtsTableMetadata::new(name, 0, EtsTableType::Set, Protection::Protected, owner)
}

#[test]
fn replay_scheduler_forces_single_worker_even_with_thread_override() {
    let scheduler = Scheduler::new_replay(
        SchedulerConfig {
            thread_count: Some(3),
            ..SchedulerConfig::default()
        },
        ReplayLog::default(),
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

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on_ready(future: ResolveFuture<'_>) -> Result<std::net::SocketAddr, ResolveError> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = future;
    match future.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("resolver test future should be ready immediately"),
    }
}

#[test]
fn cleanup_exited_process_purges_pg_membership_without_connection() {
    // A dying process must drop its pg membership locally even when no
    // distribution connection exists — the local purge runs synchronously inside
    // `cleanup_exited_process`, and the propagated leave is a no-op (no peers).
    let scheduler = Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
        .expect("scheduler should start");
    let registry = scheduler.pg_registry();
    let scope = registry.default_scope();
    let group = scheduler.shared.atom_table.intern("exiting_workers");
    // A real table entry: finalization is exactly-once, keyed on the atomic
    // process-table removal — a pid the table never knew is skipped.
    let pid = super::supervision_tests::insert_process(&scheduler.shared, 555);

    registry.join(scope, group, pid);
    assert_eq!(
        registry.local_members(scope, group),
        vec![pid],
        "pid should be a local member before exit"
    );

    cleanup_exited_process(&scheduler.shared, pid, ExitReason::Normal);

    assert!(
        registry.local_members(scope, group).is_empty(),
        "cleanup_exited_process must purge the pid's pg membership locally"
    );
    scheduler.shutdown();
}

#[test]
fn default_distribution_config_resolves_nothing() {
    assert!(SchedulerConfig::default().distribution.is_none());
    assert_eq!(SchedulerConfig::default().jit_threshold, None);

    // Honest None (spec §3.6): the default profile builds NO distribution, so
    // the config accessor is absent rather than exposing a default resolver.
    let default_scheduler =
        Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
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
fn scheduler_uses_explicit_jit_threshold() {
    let scheduler = Scheduler::new(
        SchedulerConfig {
            jit_threshold: Some(500),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .expect("scheduler should start");

    assert_eq!(scheduler.jit_profiler().current_threshold(), 500);
    scheduler.shutdown();
}

/// R1's three gating facts, pinned at the site that holds them: the JIT
/// profiling handle is present IFF the jit feature is compiled (this test IS
/// jit-compiled), replay mode is off, and the dirty-CPU service is live.
#[test]
fn replay_and_disabled_dirty_cpu_compose_the_jit_handle_away() {
    let live = Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
        .expect("live scheduler starts");
    let services =
        supervision_integration::build_native_services(&live.shared, NamespaceId::DEFAULT);
    assert!(
        services.jit_profiling.is_some(),
        "default profile with a live dirty-CPU pool must offer the profiling handle"
    );
    live.shutdown();

    let replay = Scheduler::new_replay(SchedulerConfig::default(), ReplayLog::default())
        .expect("replay scheduler starts");
    let services =
        supervision_integration::build_native_services(&replay.shared, NamespaceId::DEFAULT);
    assert!(
        services.jit_profiling.is_none(),
        "replay composes the profiling handle away entirely"
    );
    replay.shutdown();

    let minimal = Scheduler::with_services(
        SchedulerConfig::default(),
        SchedulerServices::minimal(),
        Arc::new(ModuleRegistry::new()),
    )
    .expect("minimal scheduler starts");
    let services =
        supervision_integration::build_native_services(&minimal.shared, NamespaceId::DEFAULT);
    assert!(
        services.jit_profiling.is_none(),
        "a Disabled dirty-CPU service composes the profiling handle away"
    );
    minimal.shutdown();
}

#[test]
fn failing_jit_compiler_construction_surfaces_from_the_scheduler_constructor() {
    INJECT_JIT_COMPILER_FAILURE.with(|flag| flag.set(true));
    let result = Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()));
    INJECT_JIT_COMPILER_FAILURE.with(|flag| flag.set(false));

    match result {
        Err(error) => assert!(
            error.contains("JIT compiler construction failed"),
            "constructor error must name the JIT compiler: {error}"
        ),
        Ok(scheduler) => {
            scheduler.shutdown();
            panic!("a failing JitCompiler::new must fail scheduler construction, not degrade");
        }
    }
}

#[test]
fn delete_module_drops_jit_profile_entries() {
    let scheduler = Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
        .expect("scheduler should start");
    let module = scheduler.shared.atom_table.intern("jit_deleted_module");
    let function = scheduler.shared.atom_table.intern("hot");
    // Entries are born at a live call edge; completions only update them.
    scheduler.jit_profiler().record_call(module, function, 0, 1);
    let epoch = scheduler
        .jit_profiler()
        .profile_epoch(module, function, 0)
        .expect("profile exists");
    scheduler
        .jit_profiler()
        .mark_compiled(module, function, 0, 1, epoch);
    assert!(scheduler.jit_profiler().is_compiled(module, function, 0));

    scheduler.delete_module(module);

    assert!(
        !scheduler.jit_profiler().is_compiled(module, function, 0),
        "delete_module must drop the module's JIT profile entries"
    );
    scheduler.shutdown();
}

#[test]
fn ets_registry_create_lookup_name_and_delete() {
    let scheduler = Scheduler::new(SchedulerConfig::default(), Arc::new(ModuleRegistry::new()))
        .expect("scheduler should start");
    let name = scheduler.shared.atom_table.intern("named_ets_table");

    let first_id = scheduler.shared.create_table(ets_metadata(Some(name), 99));
    let second_id = scheduler.shared.create_table(ets_metadata(None, 99));

    assert_ne!(first_id, second_id);
    assert!(second_id > first_id);
    assert_eq!(scheduler.shared.lookup_table_by_name(name), Some(first_id));

    let table = scheduler
        .shared
        .lookup_table(first_id)
        .expect("table should be present by id");
    assert_eq!(table.metadata().id, first_id);
    assert_eq!(table.metadata().name, Some(name));

    assert!(scheduler.shared.delete_table(first_id));
    assert!(scheduler.shared.lookup_table(first_id).is_none());
    assert_eq!(scheduler.shared.lookup_table_by_name(name), None);
    assert!(scheduler.shared.lookup_table(second_id).is_some());
    assert!(!scheduler.shared.delete_table(first_id));

    scheduler.shutdown();
}

fn test_module(name: Atom, code: Vec<Instruction>) -> Module {
    let label_index = code
        .iter()
        .enumerate()
        .filter_map(|(ip, instruction)| match instruction {
            Instruction::Label { label } => Some((*label, ip)),
            _ => None,
        })
        .collect();
    Module {
        name,
        generation: 0,
        origin: ModuleOrigin::Preloaded,
        exports: StdHashMap::new(),
        label_index,
        code,
        literals: Vec::new(),
        constant_pool: Default::default(),
        resolved_imports: Vec::new(),
        lambdas: Vec::new(),
        string_table: Vec::new(),
        function_table: Vec::new(),
        line_table: Vec::new(),
        line_info: Vec::new(),
    }
}

fn wait_until(deadline_ms: u64, mut predicate: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(deadline_ms);
    while !predicate() {
        assert!(std::time::Instant::now() <= deadline, "condition timed out");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(feature = "telemetry")]
fn install_telemetry_test_provider() -> (InMemorySpanExporter, SdkTracerProvider) {
    // Install the W3C trace-context propagator so injected/extracted carriers
    // actually carry the parent span; without it the global propagator is a
    // no-op and `start_process_trace_context` cannot nest under its parent.
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    (exporter, provider)
}

#[cfg(feature = "telemetry")]
fn span_attr_i64(span: &opentelemetry_sdk::trace::SpanData, key: &'static str) -> Option<i64> {
    span.attributes.iter().find_map(|attribute| {
        (attribute.key == Key::from_static_str(key)).then_some(match &attribute.value {
            opentelemetry::Value::I64(value) => Some(*value),
            _ => None,
        })?
    })
}

#[cfg(feature = "telemetry")]
fn span_attr_str(span: &opentelemetry_sdk::trace::SpanData, key: &'static str) -> Option<String> {
    span.attributes.iter().find_map(|attribute| {
        (attribute.key == Key::from_static_str(key)).then(|| match &attribute.value {
            opentelemetry::Value::String(value) => Some(value.to_string()),
            _ => None,
        })?
    })
}

#[cfg(feature = "telemetry")]
fn install_metric_test_provider() -> (InMemoryMetricExporter, SdkMeterProvider) {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    opentelemetry::global::set_meter_provider(provider.clone());
    (exporter, provider)
}

#[cfg(feature = "telemetry")]
fn find_metric<'a>(
    metrics: &'a [ResourceMetrics],
    name: &str,
) -> Option<&'a opentelemetry_sdk::metrics::data::Metric> {
    metrics
        .iter()
        .flat_map(|resource| resource.scope_metrics())
        .flat_map(|scope| scope.metrics())
        .find(|metric| metric.name() == name)
}

#[cfg(feature = "telemetry")]
fn metric_has_u64_sum_at_least(metrics: &[ResourceMetrics], name: &str, minimum: u64) -> bool {
    let Some(metric) = find_metric(metrics, name) else {
        return false;
    };
    match metric.data() {
        AggregatedMetrics::U64(MetricData::Sum(sum)) => {
            sum.data_points().any(|point| point.value() >= minimum)
        }
        _ => false,
    }
}

#[cfg(feature = "telemetry")]
fn metric_has_u64_gauge_at_least(metrics: &[ResourceMetrics], name: &str, minimum: u64) -> bool {
    let Some(metric) = find_metric(metrics, name) else {
        return false;
    };
    match metric.data() {
        AggregatedMetrics::U64(MetricData::Gauge(gauge)) => {
            gauge.data_points().any(|point| point.value() >= minimum)
        }
        _ => false,
    }
}

#[cfg(feature = "telemetry")]
fn metric_has_f64_gauge_between(
    metrics: &[ResourceMetrics],
    name: &str,
    minimum: f64,
    maximum: f64,
) -> bool {
    let Some(metric) = find_metric(metrics, name) else {
        return false;
    };
    match metric.data() {
        AggregatedMetrics::F64(MetricData::Gauge(gauge)) => gauge
            .data_points()
            .any(|point| (minimum..=maximum).contains(&point.value())),
        _ => false,
    }
}

#[cfg(feature = "telemetry")]
fn metric_has_histogram_count_at_least(
    metrics: &[ResourceMetrics],
    name: &str,
    minimum: u64,
) -> bool {
    let Some(metric) = find_metric(metrics, name) else {
        return false;
    };
    match metric.data() {
        AggregatedMetrics::F64(MetricData::Histogram(histogram)) => histogram
            .data_points()
            .any(|point| point.count() >= minimum),
        _ => false,
    }
}

#[cfg(feature = "telemetry")]
fn metric_has_string_attribute(
    metrics: &[ResourceMetrics],
    name: &str,
    key: &str,
    value: &str,
) -> bool {
    fn has_attribute<'a>(
        mut attributes: impl Iterator<Item = &'a opentelemetry::KeyValue>,
        key: &str,
        value: &str,
    ) -> bool {
        attributes.any(|attribute| {
            attribute.key.as_str() == key
                && matches!(&attribute.value, opentelemetry::Value::String(actual) if actual.to_string() == value)
        })
    }

    let Some(metric) = find_metric(metrics, name) else {
        return false;
    };
    match metric.data() {
        AggregatedMetrics::U64(MetricData::Sum(sum)) => sum
            .data_points()
            .any(|point| has_attribute(point.attributes(), key, value)),
        AggregatedMetrics::U64(MetricData::Gauge(gauge)) => gauge
            .data_points()
            .any(|point| has_attribute(point.attributes(), key, value)),
        AggregatedMetrics::F64(MetricData::Histogram(histogram)) => histogram
            .data_points()
            .any(|point| has_attribute(point.attributes(), key, value)),
        _ => false,
    }
}

#[cfg(feature = "telemetry")]
fn metric_has_pid_gauge_at_least(
    metrics: &[ResourceMetrics],
    name: &str,
    pid: u64,
    minimum: u64,
) -> bool {
    let Some(metric) = find_metric(metrics, name) else {
        return false;
    };
    let pid_i64 = i64::try_from(pid).unwrap_or(i64::MAX);
    match metric.data() {
        AggregatedMetrics::U64(MetricData::Gauge(gauge)) => gauge.data_points().any(|point| {
            point.value() >= minimum
                && point.attributes().any(|attribute| {
                    attribute.key == Key::from_static_str("pid")
                        && matches!(&attribute.value, opentelemetry::Value::I64(value) if *value == pid_i64)
                })
        }),
        _ => false,
    }
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
fn hook_records_reduction_yield_metadata_and_can_suspend_then_resume() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("hook_loop");
    let registry = Arc::new(ModuleRegistry::new());
    // A real entry is AFTER func_info (its only reach is the no-match fail edge,
    // where it now RAISES), so the happy path never runs func_info. Since
    // func_info is also current_mfa's only setter, this scaffold's hook MFA is
    // unset (NIL) — a pre-existing telemetry limitation the func_info fix exposes.
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
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let events = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let events_by_hook = Arc::clone(&events);
    let calls_by_hook = Arc::clone(&calls);
    scheduler.hook().register(move |event| {
        events_by_hook
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event);
        if calls_by_hook.fetch_add(1, Ordering::AcqRel) == 0 {
            HookDecision::Suspend
        } else {
            HookDecision::Continue
        }
    });

    let pid = scheduler.spawn_process(&module);
    wait_until(2_000, || calls.load(Ordering::Acquire) == 1);
    std::thread::sleep(std::time::Duration::from_millis(25));
    assert_eq!(
        calls.load(Ordering::Acquire),
        1,
        "suspended process is held"
    );
    assert!(scheduler.resume_process(pid));
    wait_until(2_000, || calls.load(Ordering::Acquire) > 1);

    let events = events.lock().unwrap_or_else(|error| error.into_inner());
    let first = events.first().copied().expect("hook event recorded");
    assert_eq!(first.pid, pid);
    // current_mfa's only setter is func_info, off the happy path, so the hook
    // observes the unset MFA (see the module comment + the leg-1 handoff note).
    assert_eq!(first.module, Atom::NIL);
    assert_eq!(first.function, Atom::NIL);
    assert_eq!(first.arity, 0);
    assert_eq!(first.reductions_consumed, DEFAULT_REDUCTION_BUDGET);
    drop(events);
    scheduler.shutdown();
}

#[cfg(feature = "telemetry")]
#[test]
fn execute_slice_emits_telemetry_span_with_mfa_reductions_and_outcome() {
    let _guard = crate::telemetry::test_lock::guard();
    let (exporter, provider) = install_telemetry_test_provider();
    let atoms = Arc::new(AtomTable::new());
    let module_name = atoms.intern("telemetry_slice");
    let function = atoms.intern("main");
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(test_module(
        module_name,
        vec![
            Instruction::FuncInfo {
                module: Operand::Atom(Some(module_name)),
                function: Operand::Atom(Some(function)),
                arity: Operand::Unsigned(0),
            },
            Instruction::Label { label: 1 },
            Instruction::CallOnly {
                arity: Operand::Unsigned(0),
                label: Operand::Label(1),
            },
        ],
    ));
    let scheduler = Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        Arc::clone(&atoms),
        Arc::new(BifRegistryImpl::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();

    let mut process = Process::new(44, DEFAULT_HEAP_SIZE);
    process.set_code_position(Some(CodePosition {
        module: module_name,
        instruction_pointer: 0,
    }));
    process.set_current_module(Arc::clone(&module));

    let SliceOutcome::Requeue(_) = execute_slice(&scheduler.shared, &mut process) else {
        panic!("looping process should yield after consuming its slice");
    };
    provider.force_flush().expect("spans flush");
    let spans = exporter.get_finished_spans().expect("finished spans");
    // Other scheduler tests running concurrently emit execution-slice spans
    // into the shared global tracer provider, so match on this test's own pid.
    let span = spans
        .iter()
        .find(|span| {
            span.name.as_ref() == "beamr.scheduler.execute_slice"
                && span_attr_i64(span, "process.pid") == Some(44)
        })
        .expect("execution-slice span emitted");

    assert_eq!(span_attr_i64(span, "process.pid"), Some(44));
    assert_eq!(
        span_attr_str(span, "code.module").as_deref(),
        Some("telemetry_slice")
    );
    assert_eq!(
        span_attr_str(span, "code.function").as_deref(),
        Some("main")
    );
    assert_eq!(span_attr_i64(span, "code.arity"), Some(0));
    assert_eq!(
        span_attr_i64(span, "reductions.consumed"),
        Some(i64::from(DEFAULT_REDUCTION_BUDGET))
    );
    assert_eq!(span_attr_str(span, "outcome").as_deref(), Some("yielded"));

    provider.shutdown().expect("provider shutdown");
}

#[cfg(feature = "telemetry")]
#[test]
fn spawned_process_trace_context_nests_process_and_slice_under_workflow_span() {
    let _guard = crate::telemetry::test_lock::guard();
    let (exporter, provider) = install_telemetry_test_provider();
    let atoms = Arc::new(AtomTable::new());
    let module_name = atoms.intern("telemetry_workflow_slice");
    let function = atoms.intern("main");
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(test_module(
        module_name,
        vec![
            Instruction::FuncInfo {
                module: Operand::Atom(Some(module_name)),
                function: Operand::Atom(Some(function)),
                arity: Operand::Unsigned(0),
            },
            Instruction::Label { label: 1 },
            Instruction::Return,
        ],
    ));
    let scheduler = Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        Arc::clone(&atoms),
        Arc::new(BifRegistryImpl::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();

    let tracer = opentelemetry::global::tracer("beamr-test");
    let workflow_span = tracer.start("meridian.workflow");
    let workflow_span_id = workflow_span.span_context().span_id();
    let workflow_context = opentelemetry::Context::current_with_span(workflow_span);
    let pid = 66;
    let mut process = super::spawning::build_process(super::spawning::SpawnRequest {
        pid,
        module: module_name,
        module_version: module,
        instruction_pointer: 0,
        args: Vec::new(),
        parent_pid: 0,
        function,
        arity: 0,
        capabilities: CapabilitySet::all(),
        namespace_id: NamespaceId::DEFAULT,
        group_leader: Term::pid(pid),
        priority: Priority::Normal,
        heap_size: DEFAULT_HEAP_SIZE,
        trace_context: Some(crate::telemetry::spans::inject_context(&workflow_context)),
    });

    let SliceOutcome::Exited(ExitReason::Normal, _) =
        execute_slice(&scheduler.shared, &mut process)
    else {
        panic!("workflow step process should exit normally");
    };
    workflow_context.span().end();
    provider.force_flush().expect("spans flush");
    let spans = exporter.get_finished_spans().expect("finished spans");
    let workflow = spans
        .iter()
        .find(|span| span.name.as_ref() == "meridian.workflow")
        .expect("workflow span emitted");
    // Other scheduler tests running concurrently emit process and
    // execution-slice spans into the shared global tracer provider, so match
    // on this test's own pid (the pid-44 sibling above set the idiom; the
    // name-only form of these selections flaked under a parallel full run).
    let process = spans
        .iter()
        .find(|span| {
            span.name.as_ref() == "beamr.process" && span_attr_i64(span, "process.pid") == Some(66)
        })
        .expect("process span emitted");
    let slice = spans
        .iter()
        .find(|span| {
            span.name.as_ref() == "beamr.scheduler.execute_slice"
                && span_attr_i64(span, "process.pid") == Some(66)
        })
        .expect("execution-slice span emitted");

    assert_eq!(workflow.span_context.span_id(), workflow_span_id);
    assert_eq!(process.parent_span_id, workflow.span_context.span_id());
    assert_eq!(slice.parent_span_id, process.span_context.span_id());
    assert_eq!(
        span_attr_i64(process, "process.pid"),
        Some(i64::try_from(pid).unwrap_or(i64::MAX))
    );
    assert_eq!(
        span_attr_i64(slice, "process.pid"),
        Some(i64::try_from(pid).unwrap_or(i64::MAX))
    );
    assert_eq!(
        span_attr_str(process, "process.exit_reason").as_deref(),
        Some("normal")
    );

    provider.shutdown().expect("provider shutdown");
}

/// Deterministic isolation wall for the pid-narrowed span selections above
/// (mirrors the lifecycle instance-recorder storm wall): with the test
/// provider installed globally, a storm thread floods the shared tracer with
/// foreign process/execution-slice spans under a foreign pid, and the
/// pid-narrowed selection must still resolve exactly this test's own spans
/// and their parent links. Removing a pid filter from the selection idiom
/// turns this wall red.
#[cfg(feature = "telemetry")]
#[test]
fn pid_narrowed_span_selection_survives_foreign_execute_slice_spans() {
    let _guard = crate::telemetry::test_lock::guard();
    let (exporter, provider) = install_telemetry_test_provider();

    // Foreign storm: another test's scheduler work, reduced to its telemetry
    // signature -- same span names, different pid -- emitted through the same
    // global tracer this test's exporter now backs. Joined before selection,
    // so contamination is deterministic, not timing-dependent.
    let storm = std::thread::spawn(|| {
        let tracer = opentelemetry::global::tracer("beamr");
        for _ in 0..64 {
            let mut foreign = tracer.start("beamr.scheduler.execute_slice");
            foreign.set_attribute(opentelemetry::KeyValue::new("process.pid", 9_999_i64));
            foreign.end();
            let mut foreign_process = tracer.start("beamr.process");
            foreign_process.set_attribute(opentelemetry::KeyValue::new("process.pid", 9_999_i64));
            foreign_process.end();
        }
    });
    storm.join().expect("storm thread completes");

    let atoms = Arc::new(AtomTable::new());
    let module_name = atoms.intern("telemetry_isolation_wall");
    let function = atoms.intern("main");
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(test_module(
        module_name,
        vec![
            Instruction::FuncInfo {
                module: Operand::Atom(Some(module_name)),
                function: Operand::Atom(Some(function)),
                arity: Operand::Unsigned(0),
            },
            Instruction::Label { label: 1 },
            Instruction::Return,
        ],
    ));
    let scheduler = Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        Arc::clone(&atoms),
        Arc::new(BifRegistryImpl::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();

    let tracer = opentelemetry::global::tracer("beamr-test");
    let workflow_span = tracer.start("meridian.workflow");
    let workflow_span_id = workflow_span.span_context().span_id();
    let workflow_context = opentelemetry::Context::current_with_span(workflow_span);
    let pid = 67;
    let mut owned = super::spawning::build_process(super::spawning::SpawnRequest {
        pid,
        module: module_name,
        module_version: module,
        instruction_pointer: 0,
        args: Vec::new(),
        parent_pid: 0,
        function,
        arity: 0,
        capabilities: CapabilitySet::all(),
        namespace_id: NamespaceId::DEFAULT,
        group_leader: Term::pid(pid),
        priority: Priority::Normal,
        heap_size: DEFAULT_HEAP_SIZE,
        trace_context: Some(crate::telemetry::spans::inject_context(&workflow_context)),
    });

    let SliceOutcome::Exited(ExitReason::Normal, _) = execute_slice(&scheduler.shared, &mut owned)
    else {
        panic!("isolation-wall process should exit normally");
    };
    workflow_context.span().end();
    provider.force_flush().expect("spans flush");
    let spans = exporter.get_finished_spans().expect("finished spans");

    // The contamination is really present -- the wall proves selection under
    // pollution, not a clean exporter.
    let foreign_slices = spans
        .iter()
        .filter(|span| {
            span.name.as_ref() == "beamr.scheduler.execute_slice"
                && span_attr_i64(span, "process.pid") == Some(9_999)
        })
        .count();
    assert_eq!(
        foreign_slices, 64,
        "foreign storm spans present in exporter"
    );

    let process = spans
        .iter()
        .find(|span| {
            span.name.as_ref() == "beamr.process" && span_attr_i64(span, "process.pid") == Some(67)
        })
        .expect("owned process span resolves under pollution");
    let slice = spans
        .iter()
        .find(|span| {
            span.name.as_ref() == "beamr.scheduler.execute_slice"
                && span_attr_i64(span, "process.pid") == Some(67)
        })
        .expect("owned execution-slice span resolves under pollution");

    assert_eq!(process.parent_span_id, workflow_span_id);
    assert_eq!(slice.parent_span_id, process.span_context.span_id());

    provider.shutdown().expect("provider shutdown");
}

#[cfg(feature = "telemetry")]
#[test]
fn execute_slice_emits_vm_health_and_process_metrics() {
    let _guard = crate::telemetry::test_lock::guard();
    let (exporter, provider) = install_metric_test_provider();
    let atoms = Arc::new(AtomTable::new());
    let module_name = atoms.intern("telemetry_metrics_slice");
    let function = atoms.intern("main");
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(test_module(
        module_name,
        vec![
            Instruction::FuncInfo {
                module: Operand::Atom(Some(module_name)),
                function: Operand::Atom(Some(function)),
                arity: Operand::Unsigned(0),
            },
            Instruction::Label { label: 1 },
            Instruction::CallOnly {
                arity: Operand::Unsigned(0),
                label: Operand::Label(1),
            },
        ],
    ));
    let scheduler = Scheduler::with_code_server(
        SchedulerConfig {
            thread_count: Some(1),
            telemetry_sample_interval: Some(std::time::Duration::ZERO),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
        Arc::clone(&atoms),
        Arc::new(BifRegistryImpl::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();

    // Register the process in the scheduler's table so the alive-process
    // gauge (`process_count() == process_table.len()`) reflects a real live
    // process rather than zero.
    scheduler.shared.process_table.spawn_with_pid(55);
    let mut process = Process::new(55, DEFAULT_HEAP_SIZE);
    process.set_code_position(Some(CodePosition {
        module: module_name,
        instruction_pointer: 0,
    }));
    process.set_current_module(Arc::clone(&module));
    for index in 0..5 {
        process
            .mailbox_mut()
            .push_owned_for_test(Term::small_int(index));
    }

    let SliceOutcome::Requeue(_) = execute_slice(&scheduler.shared, &mut process) else {
        panic!("looping process should yield after consuming its slice");
    };

    crate::telemetry::metrics::record_gc_collection("minor", std::time::Duration::from_micros(50));
    crate::telemetry::metrics::record_message_sent();
    crate::telemetry::metrics::record_workflow_started("workflow-123");
    crate::telemetry::metrics::record_workflow_step_completed(
        "workflow-123",
        "function",
        std::time::Duration::from_millis(25),
    );
    provider.force_flush().expect("metrics flush");
    let metrics = exporter.get_finished_metrics().expect("finished metrics");

    assert!(metric_has_u64_gauge_at_least(
        &metrics,
        "beamr.processes.alive",
        1
    ));
    assert!(metric_has_f64_gauge_between(
        &metrics,
        "beamr.scheduler.utilization",
        0.0,
        1.0
    ));
    assert!(metric_has_u64_sum_at_least(
        &metrics,
        "beamr.gc.collections",
        1
    ));
    assert!(metric_has_histogram_count_at_least(
        &metrics,
        "beamr.gc.duration",
        1
    ));
    assert!(metric_has_u64_sum_at_least(
        &metrics,
        "beamr.messages.sent",
        1
    ));
    assert!(find_metric(&metrics, "beamr.memory.heap_words").is_some());
    assert!(metric_has_u64_sum_at_least(
        &metrics,
        "beamr.process.reductions",
        u64::from(DEFAULT_REDUCTION_BUDGET)
    ));
    assert!(metric_has_pid_gauge_at_least(
        &metrics,
        "beamr.process.message_queue_len",
        55,
        5
    ));

    assert!(metric_has_u64_sum_at_least(
        &metrics,
        "beamr.workflow.steps_completed",
        1
    ));
    assert!(metric_has_histogram_count_at_least(
        &metrics,
        "beamr.workflow.step_duration",
        1
    ));
    assert!(find_metric(&metrics, "beamr.workflow.active").is_some());
    assert!(metric_has_string_attribute(
        &metrics,
        "beamr.workflow.steps_completed",
        "workflow_id",
        "workflow-123"
    ));
    assert!(metric_has_string_attribute(
        &metrics,
        "beamr.workflow.step_duration",
        "step_type",
        "function"
    ));
    assert!(metric_has_string_attribute(
        &metrics,
        "beamr.workflow.active",
        "workflow_id",
        "workflow-123"
    ));

    crate::telemetry::metrics::record_workflow_finished("workflow-123");
    provider
        .force_flush()
        .expect("metrics flush after workflow finish");

    provider.shutdown().expect("provider shutdown");
}

#[test]
fn hook_fires_when_process_blocks_on_receive() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("hook_wait");
    let registry = Arc::new(ModuleRegistry::new());
    // Entry is after func_info (which now RAISES on the no-match fail edge), so
    // the happy path is the receive body only.
    let module = test_module(
        module_name,
        vec![
            Instruction::Label { label: 10 },
            Instruction::Wait {
                fail: Operand::Label(10),
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
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_by_hook = Arc::clone(&events);
    scheduler.hook().register(move |event| {
        events_by_hook
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event);
        HookDecision::Continue
    });

    let pid = scheduler.spawn_process(&module);
    wait_until(2_000, || {
        !events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
    });
    let events = events.lock().unwrap_or_else(|error| error.into_inner());
    assert_eq!(events[0].pid, pid);
    // current_mfa's only setter is func_info, off the happy path, so the hook
    // observes the unset MFA (NIL). See the leg-1 handoff note.
    assert_eq!(events[0].module, Atom::NIL);
    assert_eq!(events[0].function, Atom::NIL);
    assert_eq!(events[0].arity, 0);
    drop(events);
    scheduler.shutdown();
}

#[test]
fn scheduler_default_thread_count_matches_available_parallelism() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(SchedulerConfig::default(), registry)
        .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let expected = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    assert_eq!(scheduler.thread_count(), expected);

    scheduler.shutdown();
}

#[test]
fn shutdown_is_idempotent() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(2),
            ..SchedulerConfig::default()
        },
        registry,
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
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let pid = scheduler.spawn_process(&module);

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
        process_bodies: DashMap::new(),
        exit_tombstones: exit_tombstones::BoundedTombstones::new(),
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
        atom_table,
        ets_registry: Arc::new(crate::ets::EtsRegistry::new()),
        pg_registry: Arc::new(crate::distribution::pg::PgRegistry::new(
            &crate::atom::AtomTable::with_common_atoms(),
        )),
        bif_registry: Arc::new(crate::native::BifRegistryImpl::new()),
        capability_policy: Arc::new(crate::native::AllCapabilitiesPolicy),
        idle_parks: AtomicUsize::new(0),
        observed_park_timeout_millis: AtomicU64::new(0),
        suspension_mirror_registrations: AtomicU64::new(0),
        dirty_suspension_allocations: AtomicU64::new(0),
        park_gap_hook: Mutex::new(None),
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
fn linked_test_spawn_inherits_parent_group_leader_not_child_pid() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        registry,
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();
    let parent = scheduler.spawn_test_process(false);
    let parent_group_leader = Term::pid(77);
    assert!(scheduler.set_test_group_leader(parent, parent_group_leader));

    let child = scheduler
        .spawn_linked_test_process(parent)
        .unwrap_or_else(|error| panic!("linked child starts: {error}"));

    assert_eq!(
        scheduler.test_group_leader(child),
        Some(parent_group_leader)
    );
    assert_ne!(scheduler.test_group_leader(child), Some(Term::pid(child)));
}

#[test]
fn spawn_link_uses_executing_parent_namespace_and_merges_parent_link() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("spawn_link_child");
    let function = atoms.intern("main");
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let namespace = scheduler.create_namespace();
    let namespace_registry = scheduler
        .shared
        .namespace_store
        .get(&namespace)
        .map(|entry| Arc::clone(&entry))
        .unwrap_or_else(|| panic!("namespace registry exists"));
    // The child PARKS on Wait (label 7): its body persists for the namespace
    // and link assertions without racing the live worker. (This test formerly
    // used shutdown-as-barrier and spawned afterwards; post-shutdown spawns
    // are now REFUSED by the §4 teardown gate, so the barrier is the parked
    // child instead.)
    let mut module = test_module(
        module_name,
        vec![
            Instruction::Label { label: 7 },
            Instruction::Wait {
                fail: Operand::Label(7),
            },
        ],
    );
    module.exports.insert((function, 0), 7);
    let module = namespace_registry.insert(module);
    let parent = scheduler.spawn_test_process_in(namespace, Arc::clone(&module));

    let process = take_runnable_process(&scheduler.shared, parent)
        .unwrap_or_else(|| panic!("parent body taken"));

    let child = scheduler
        .spawn_link(parent, module_name, function, Vec::new())
        .unwrap_or_else(|error| panic!("spawn_link succeeds with executing parent: {error:?}"));

    assert_eq!(scheduler.process_namespace(parent), Some(namespace));
    assert_eq!(scheduler.process_namespace(child), Some(namespace));
    assert!(process_links_contain(&scheduler.shared, parent, child));
    store_runnable_process(&scheduler.shared, process);
    assert!(scheduler.is_linked(parent, child));
    scheduler.shutdown();
}

/// Wait (bounded) until `pid` is Present and parked Waiting — used by tests
/// whose children park on a `Wait` instruction so their bodies stay stably
/// inspectable with the worker live (post-shutdown spawns are refused by the
/// §4 teardown gate, so shutdown-as-barrier-then-spawn is no longer legal).
fn wait_until_parked(shared: &SharedState, pid: u64) {
    wait_until(10_000, || {
        shared.process_bodies.get(&pid).is_some_and(|entry| {
            let slot = lock_or_recover(&entry);
            matches!(
                &*slot,
                ProcessSlot::Present(ScheduledProcess(process))
                    if process.status() == ProcessStatus::Waiting
            )
        })
    });
}

#[test]
fn spawn_facility_options_apply_link_monitor_priority_and_heap_before_wake() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("spawn_opt_scheduler");
    let function = atoms.intern("main");
    let mut module = test_module(
        module_name,
        vec![
            Instruction::Label { label: 7 },
            Instruction::Wait {
                fail: Operand::Label(7),
            },
        ],
    );
    module.exports.insert((function, 0), 7);
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let parent = scheduler.spawn_test_process_in(NamespaceId::DEFAULT, Arc::clone(&module));
    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };

    let result = facility
        .spawn_with_options(
            parent,
            module_name,
            function,
            Vec::new(),
            SpawnOptions {
                link: true,
                monitor: true,
                priority: Some(Priority::High),
                min_heap_size: Some(512),
                capabilities: None,
            },
        )
        .unwrap_or_else(|error| panic!("spawn_with_options succeeds: {error:?}"));
    // The child parks on Wait; from then on its body is stably Present.
    wait_until_parked(&scheduler.shared, result.pid);

    assert!(scheduler.is_linked(parent, result.pid));
    let parent_entry = scheduler
        .shared
        .process_bodies
        .get(&parent)
        .unwrap_or_else(|| panic!("parent body exists"));
    let parent_slot = lock_or_recover(&parent_entry);
    let ProcessSlot::Present(ScheduledProcess(parent_process)) = &*parent_slot else {
        panic!("parent process should be present");
    };
    let reference = result.reference.expect("monitor reference");
    assert!(parent_process.links().contains(&result.pid));
    assert!(
        parent_process
            .monitors()
            .iter()
            .any(|monitor| monitor.reference() == reference
                && monitor.watcher() == parent
                && monitor.target() == result.pid)
    );
    drop(parent_slot);
    drop(parent_entry);

    let child_entry = scheduler
        .shared
        .process_bodies
        .get(&result.pid)
        .unwrap_or_else(|| panic!("child body exists"));
    let child_slot = lock_or_recover(&child_entry);
    let ProcessSlot::Present(ScheduledProcess(child_process)) = &*child_slot else {
        panic!("child process should be present");
    };
    assert_eq!(child_process.priority(), Priority::High);
    assert_eq!(child_process.heap().capacity(), 512);
    assert!(child_process.links().contains(&parent));
    assert!(
        child_process
            .monitors()
            .iter()
            .any(|monitor| monitor.reference() == reference
                && monitor.watcher() == parent
                && monitor.target() == result.pid)
    );
    drop(child_slot);
    drop(child_entry);
    scheduler.shutdown();
}

#[test]
fn spawn_facility_restricts_child_to_explicit_capabilities() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("spawn_opt_capability_scheduler");
    let function = atoms.intern("main");
    let mut module = test_module(
        module_name,
        vec![
            Instruction::Label { label: 7 },
            Instruction::Wait {
                fail: Operand::Label(7),
            },
        ],
    );
    module.exports.insert((function, 0), 7);
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let parent = scheduler.spawn_test_process_in(NamespaceId::DEFAULT, Arc::clone(&module));
    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };
    let restricted = CapabilitySet::from_slice(&[Capability::Pure, Capability::ProcessLocal]);

    let result = facility
        .spawn_with_options(
            parent,
            module_name,
            function,
            Vec::new(),
            SpawnOptions {
                capabilities: Some(restricted.clone()),
                ..SpawnOptions::default()
            },
        )
        .unwrap_or_else(|error| panic!("spawn_with_options succeeds: {error:?}"));
    // The child parks on Wait; from then on its body is stably Present.
    wait_until_parked(&scheduler.shared, result.pid);

    let child_entry = scheduler
        .shared
        .process_bodies
        .get(&result.pid)
        .unwrap_or_else(|| panic!("child body exists"));
    let child_slot = lock_or_recover(&child_entry);
    let ProcessSlot::Present(ScheduledProcess(child_process)) = &*child_slot else {
        panic!("child process should be present");
    };
    assert_eq!(child_process.capabilities(), &restricted);
    assert!(
        !child_process
            .capabilities()
            .contains(Capability::ExternalIo)
    );
}

#[test]
fn process_info_reads_executing_process_metadata() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("executing_info");
    let function = atoms.intern("main");
    let module = test_module(module_name, vec![Instruction::Label { label: 1 }]);
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();
    let pid = scheduler.spawn_test_process_in(NamespaceId::DEFAULT, Arc::clone(&module));
    {
        let entry = scheduler
            .shared
            .process_bodies
            .get(&pid)
            .unwrap_or_else(|| panic!("process body exists"));
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
            panic!("test process should be present");
        };
        process.set_current_mfa(Some((module_name, function, 0)));
    }

    let process = take_runnable_process(&scheduler.shared, pid)
        .unwrap_or_else(|| panic!("process should transition to executing"));

    assert_eq!(
        scheduler
            .shared
            .process_info(pid, ProcessInfoItem::CurrentFunction),
        Some(ProcessInfoValue::CurrentFunction(Some((
            module_name,
            function,
            0
        ))))
    );
    assert_eq!(
        scheduler.shared.process_info(pid, ProcessInfoItem::Status),
        Some(ProcessInfoValue::Status(ProcessInfoStatus::Running))
    );

    store_runnable_process(&scheduler.shared, process);
}

#[test]
fn process_info_current_function_is_derived_from_module_and_ip() {
    // Fail-first (current_mfa lane #1): a process parked mid-function carries
    // `current_module` + `code_position` but NO stored MFA (func_info, its only
    // writer, runs only on dispatch failure). process_info(current_function)
    // must DERIVE the MFA from the module's func_info bounds at the current ip.
    // RED on `main`: the stale/None stored field answers `{undefined,undefined,0}`.
    let atoms = AtomTable::new();
    let module_name = atoms.intern("parked_mod");
    let waiter = atoms.intern("waiter");
    // A module whose func_info table bounds `waiter/0` from ip 0 onward. The
    // process is positioned at ip 1 (inside those bounds), as a receive-parked
    // process would be between scheduler slices.
    let mut module = test_module(
        module_name,
        vec![
            Instruction::FuncInfo {
                module: Operand::Atom(Some(module_name)),
                function: Operand::Atom(Some(waiter)),
                arity: Operand::Unsigned(0),
            },
            Instruction::Label { label: 1 },
        ],
    );
    module.function_table = vec![(0, waiter, 0)];
    let registry = Arc::new(ModuleRegistry::new());
    let module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();
    let pid = scheduler.spawn_test_process_in(NamespaceId::DEFAULT, Arc::clone(&module));
    {
        let entry = scheduler
            .shared
            .process_bodies
            .get(&pid)
            .unwrap_or_else(|| panic!("process body exists"));
        let mut slot = lock_or_recover(&entry);
        let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
            panic!("test process should be present");
        };
        process.set_code_position(Some(CodePosition {
            module: module_name,
            instruction_pointer: 1,
        }));
    }

    assert_eq!(
        scheduler
            .shared
            .process_info(pid, ProcessInfoItem::CurrentFunction),
        Some(ProcessInfoValue::CurrentFunction(Some((
            module_name,
            waiter,
            0
        )))),
        "current_function must derive `waiter/0` from (module, ip), not answer undefined"
    );
}

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
        process_bodies: DashMap::new(),
        exit_tombstones: exit_tombstones::BoundedTombstones::new(),
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
        atom_table,
        ets_registry: Arc::new(crate::ets::EtsRegistry::new()),
        pg_registry: Arc::new(crate::distribution::pg::PgRegistry::new(
            &crate::atom::AtomTable::with_common_atoms(),
        )),
        bif_registry: Arc::new(crate::native::BifRegistryImpl::new()),
        capability_policy: Arc::new(crate::native::AllCapabilitiesPolicy),
        idle_parks: AtomicUsize::new(0),
        observed_park_timeout_millis: AtomicU64::new(0),
        suspension_mirror_registrations: AtomicU64::new(0),
        dirty_suspension_allocations: AtomicU64::new(0),
        park_gap_hook: Mutex::new(None),
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
            links: Vec::new(),
            remote_links: Vec::new(),
            monitors: Vec::new(),
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
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let pid = scheduler.spawn_process(&module);
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
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let pids: Vec<_> = (0..20).map(|_| scheduler.spawn_process(&module)).collect();

    wait_until(3_000, || {
        pids.iter()
            .all(|pid| scheduler.process_table().get(*pid).is_none())
    });
    scheduler.shutdown();
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
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    wait_until(500, || scheduler.idle_park_count() > 0);
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// Park-gap interleavings: deliveries and dirty resumes racing run_process's
// park sequences. The park_gap_hook runs synchronously at the exact gap, so
// each test pins one interleaving deterministically.
// ---------------------------------------------------------------------------

fn receive_then_return_module(registry: &ModuleRegistry, name: Atom) -> Arc<Module> {
    registry.insert(test_module(
        name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::LoopRec {
                fail: Operand::Label(2),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label { label: 2 },
            Instruction::Wait {
                fail: Operand::Label(1),
            },
        ],
    ))
}

/// Pushes an atom message exactly the way a racing sender does: under the
/// slot lock, with `wake_process` left to the caller.
fn deliver_atom(shared: &SharedState, pid: u64, atom: Atom) {
    let entry = shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("process body exists"));
    let mut slot = lock_or_recover(&entry);
    match &mut *slot {
        ProcessSlot::Present(scheduled) => scheduled.0.mailbox_mut().push_owned(Term::atom(atom)),
        ProcessSlot::Executing(metadata) => metadata
            .pending_io_messages
            .push(PendingMailboxMessage::TargetOwned(Term::atom(atom))),
        ProcessSlot::Absent => panic!("process body absent"),
    }
}

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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

#[test]
fn dirty_resume_in_the_suspend_park_gap_is_not_lost() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("suspend_park_gap");
    let registry = Arc::new(ModuleRegistry::new());
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
    let module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
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

    let pid = scheduler.spawn_process(&module);
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

    let dirty_pid = scheduler.spawn_process(&dirty_module);

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
    let peer_pid = scheduler.spawn_process(&peer_module);
    let (peer_reason, _peer_result) = scheduler.run_until_exit(peer_pid);
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
    let scheduler = Scheduler::new_replay(SchedulerConfig::default(), ReplayLog::default())
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

// ---------------------------------------------------------------------------
// Receive-timer expiry racing the wait-arm park gaps: the timer flavor of the
// lost-wakeup interleavings above. expire_timers only marks and wakes, so a
// fire that lands before wait-set registration relies entirely on the Wait
// arm's post-registration recheck noticing the mark.
// ---------------------------------------------------------------------------

/// `receive ... after 1 -> true end` in the erlc instruction shape: the
/// wait_timeout fail label is the receive loop, and timer expiry falls
/// through to `timeout` and the after-body.
fn receive_after_module(registry: &ModuleRegistry, name: Atom) -> Arc<Module> {
    registry.insert(test_module(
        name,
        vec![
            Instruction::Label { label: 1 },
            Instruction::LoopRec {
                fail: Operand::Label(2),
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label { label: 2 },
            Instruction::WaitTimeout {
                fail: Operand::Label(1),
                timeout: Operand::Unsigned(1),
            },
            Instruction::Timeout,
            Instruction::Move {
                source: Operand::Atom(Some(Atom::TRUE)),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
    ))
}

/// Sleeps past the 1ms receive deadline and force-ticks the shared wheel so
/// the timer fires synchronously inside the park gap.
fn fire_receive_timer_in_gap(shared: &SharedState) {
    std::thread::sleep(std::time::Duration::from_millis(5));
    timer_integration::tick_timers(shared);
}

#[test]
fn timer_expiry_in_the_wait_park_gap_is_not_a_lost_timeout() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("timer_wait_park_gap");
    let registry = Arc::new(ModuleRegistry::new());
    let module = receive_after_module(&registry, module_name);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    // Fire the timer in the gap between the Wait arm's store-back and its
    // wait-set registration: expire_timers finds nothing in `waiting`, so
    // only the post-registration recheck (noticing the pending mark) can
    // schedule the process; the next slice consumes the mark and jumps to
    // the timeout continuation.
    let fired = Arc::new(AtomicBool::new(false));
    let fired_by_hook = Arc::clone(&fired);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap != ParkGap::WaitStored
            || pid == shared.standard_io_pid
            || fired_by_hook.swap(true, Ordering::AcqRel)
        {
            return;
        }
        fire_receive_timer_in_gap(shared);
    }));

    let pid = scheduler.spawn_process(&module);
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert!(fired.load(Ordering::Acquire), "park gap was exercised");
    let exit_value = scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root());
    assert_eq!(exit_value, Some(Term::atom(Atom::TRUE)));
    scheduler.shutdown();
}

#[test]
fn deliver_timer_pushes_message_into_target_mailbox() {
    // A `Deliver` timer (the kind backing send_after/start_timer and native
    // ctx.schedule) must deposit its message into the target's mailbox when it
    // fires. Driven deterministically with explicit instants (schedule_at +
    // tick_at), so the expired set is exact and the live scheduler thread plays
    // no part — no wall-clock sleep, no poll loop, no race on `current_tick`.
    //
    // Falsifiable: before this change `expire_timers` ONLY ran the
    // receive-timeout mark-and-wake path and never delivered
    // `ExpiredTimer.message` to any mailbox, so this assertion
    // (has_message == Some(true)) could never hold.
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let target = scheduler.spawn_test_process(false);
    let payload = Term::small_int(4242);
    assert_eq!(
        scheduler.has_message(target, payload),
        Some(false),
        "no message before the timer fires"
    );

    // Drive an isolated local wheel with explicit instants so the live
    // scheduler worker (which ticks the SHARED wheel on wall-clock) cannot race
    // the expired set. The wheel that produced the expiry is irrelevant to
    // delivery — only the recorded ExpiredTimer values matter.
    let base = std::time::Instant::now();
    let mut wheel = crate::timer::TimerWheel::new();
    let _reference = wheel.schedule_at(
        base,
        std::time::Duration::from_millis(30),
        target,
        payload,
        crate::timer::TimerKind::Deliver,
    );
    // Not yet due at +29ms; due at +30ms.
    assert!(
        wheel
            .tick_at(base + std::time::Duration::from_millis(29))
            .is_empty(),
        "the timer must not fire before its delay elapses"
    );
    let expired = wheel.tick_at(base + std::time::Duration::from_millis(30));
    assert_eq!(expired.len(), 1, "exactly one timer fires at +30ms");

    timer_integration::expire_timers_for_test(&scheduler.shared, expired);

    assert_eq!(
        scheduler.has_message(target, payload),
        Some(true),
        "a fired Deliver timer must push its message into the target mailbox"
    );
    scheduler.shutdown();
}

#[test]
fn receive_timeout_timer_does_not_deliver_a_message() {
    // Regression guard for the receive-timeout path: a ReceiveTimeout timer
    // firing must NOT push its message into the mailbox (it only marks the
    // fired-timer set for the code-position jump). Pairs with the Deliver test
    // to prove the two kinds route differently.
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let target = scheduler.spawn_test_process(false);
    let payload = Term::small_int(7);
    // Isolated local wheel (see deliver test) so the worker cannot race us.
    let base = std::time::Instant::now();
    let mut wheel = crate::timer::TimerWheel::new();
    let _reference = wheel.schedule_at(
        base,
        std::time::Duration::from_millis(1),
        target,
        payload,
        crate::timer::TimerKind::ReceiveTimeout,
    );
    let expired = wheel.tick_at(base + std::time::Duration::from_millis(1));
    assert_eq!(expired.len(), 1, "the receive-timeout timer fires");

    timer_integration::expire_timers_for_test(&scheduler.shared, expired);

    assert_eq!(
        scheduler.has_message(target, payload),
        Some(false),
        "a ReceiveTimeout timer must never deliver its message to the mailbox"
    );
    // The fired-timer mark is recorded instead (the receive-timeout path).
    assert!(
        scheduler
            .shared
            .expired_receive_timers
            .contains_key(&target),
        "a ReceiveTimeout fire records a mark for the target"
    );
    scheduler.shutdown();
}

#[test]
fn timer_expiry_after_wait_registration_schedules_the_process_once() {
    let atoms = AtomTable::new();
    let module_name = atoms.intern("timer_wait_park_gap_late");
    let registry = Arc::new(ModuleRegistry::new());
    let module = receive_after_module(&registry, module_name);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    // Fire the timer between registration and the recheck: the wake moves
    // the pid to `woken`, and the recheck's self-wake must back off (its
    // `waiting` removal finds nothing) so the process is scheduled exactly
    // once.
    let fired = Arc::new(AtomicBool::new(false));
    let fired_by_hook = Arc::clone(&fired);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap != ParkGap::WaitRegistered
            || pid == shared.standard_io_pid
            || fired_by_hook.swap(true, Ordering::AcqRel)
        {
            return;
        }
        fire_receive_timer_in_gap(shared);
    }));

    let pid = scheduler.spawn_process(&module);
    wait_until(10_000, || {
        scheduler.shared.exit_tombstones.contains_key(&pid)
    });
    assert!(fired.load(Ordering::Acquire), "park gap was exercised");
    let exit_value = scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root());
    assert_eq!(exit_value, Some(Term::atom(Atom::TRUE)));
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// Call-identity-gated suspension protocol: results are keyed by (pid, call
// id) and applied only at the suspension that produced them; gated host
// awaits are never woken (or re-executed) by plain messages; resumes are
// identity-gated and sticky.
// ---------------------------------------------------------------------------

fn native_call_module(
    registry: &ModuleRegistry,
    name: Atom,
    function: crate::native::NativeFn,
    dirty_kind: Option<crate::scheduler::DirtySchedulerKind>,
) -> Arc<Module> {
    let mut module = test_module(
        name,
        vec![
            Instruction::CallExt {
                arity: Operand::Unsigned(0),
                import: Operand::Unsigned(0),
            },
            Instruction::Return,
        ],
    );
    module.resolved_imports.push(crate::module::ResolvedImport {
        module: name,
        function: name,
        arity: 0,
        target: crate::module::ResolvedImportTarget::Native(crate::native::NativeEntry {
            function,
            dirty_kind,
            capability: Capability::Pure,
        }),
    });
    registry.insert(module)
}

fn single_thread_scheduler(registry: &Arc<ModuleRegistry>) -> Scheduler {
    Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::clone(registry),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"))
}

fn exit_value(scheduler: &Scheduler, pid: u64) -> Option<Term> {
    scheduler
        .shared
        .exit_results
        .get(&pid)
        .map(|result| result.root())
}

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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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

    let pid = scheduler.spawn_process(&module);
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
    let (reason, _value) = scheduler.run_until_exit(pid);
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

    let pid = scheduler.spawn_process(&module);
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
    let (reason, _value) = scheduler.run_until_exit(pid);
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

// --- Native busy-poll run-queue fairness (live message-loss defect) ---
//
// A native handler that drains its mailbox and returns
// `NativeOutcome::Continue` every slice (the liminal ConnectionProcess shape:
// drain `ctx.recv()`, poll a non-blocking socket, requeue on WouldBlock) is
// permanently runnable. Under a LIFO owner pop, the Requeue arm pushed the
// just-run pid on top of its own queue and the very next `queue.pop()` took
// it right back, so every OTHER pid on that thread's queue never got a
// slice. A message enqueued to such a starved process was delivered into its
// mailbox (`enqueue_atom_message` returned true) but never observed:
// `wake_process` is a no-op for a runnable pid (it is not in the wait set),
// and the steal path cannot rescue it either (mid-slice the owner's queue
// holds a single pid, which `steal_half_from` refuses to steal). These tests
// pin the FIFO owner pop that makes co-resident runnable processes
// round-robin.

struct BusyPollHandler {
    received: Arc<Mutex<Vec<Term>>>,
    slices: Arc<AtomicUsize>,
}

impl crate::native::native_process::NativeHandler for BusyPollHandler {
    fn handle(
        &mut self,
        ctx: &mut crate::native::native_process::NativeContext<'_>,
    ) -> crate::native::native_process::NativeOutcome {
        self.slices.fetch_add(1, Ordering::AcqRel);
        while let Some(message) = ctx.recv() {
            lock_or_recover(&self.received).push(message);
        }
        crate::native::native_process::NativeOutcome::Continue
    }
}

type BusyPoller = (u64, Arc<Mutex<Vec<Term>>>, Arc<AtomicUsize>);

fn spawn_busy_pollers(scheduler: &Scheduler, count: usize) -> Vec<BusyPoller> {
    (0..count)
        .map(|_| {
            let received = Arc::new(Mutex::new(Vec::new()));
            let slices = Arc::new(AtomicUsize::new(0));
            let factory_received = Arc::clone(&received);
            let factory_slices = Arc::clone(&slices);
            let pid = scheduler
                .spawn_native(Box::new(move || {
                    Box::new(BusyPollHandler {
                        received: Arc::clone(&factory_received),
                        slices: Arc::clone(&factory_slices),
                    })
                }))
                .unwrap_or_else(|error| panic!("spawn native busy poller: {error:?}"));
            (pid, received, slices)
        })
        .collect()
}

fn all_observed_atom(pollers: &[BusyPoller], atom: Atom) -> bool {
    pollers
        .iter()
        .all(|(_, received, _)| lock_or_recover(received).contains(&Term::atom(atom)))
}

#[test]
fn message_to_a_colocated_busy_poll_native_process_is_observed() {
    // One scheduler thread owning TWO permanently-runnable native processes:
    // the exact starvation shape. The message must reach both handlers.
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = single_thread_scheduler(&registry);
    let pollers = spawn_busy_pollers(&scheduler, 2);

    // Steady state: the thread is running busy-poll slices.
    wait_until(10_000, || {
        pollers
            .iter()
            .any(|(_, _, slices)| slices.load(Ordering::Acquire) > 100)
    });

    for (pid, _, _) in &pollers {
        assert!(
            scheduler.enqueue_atom_message(*pid, Atom::OK),
            "enqueue to live busy poller must succeed"
        );
    }
    wait_until(2_000, || all_observed_atom(&pollers, Atom::OK));
    scheduler.shutdown();
}

#[test]
fn messages_to_saturated_busy_poll_native_processes_are_all_observed() {
    // More permanently-runnable native processes than scheduler threads, so
    // every thread owns at least two and no thread ever idles (the saturated
    // production shape: no thread parks, so nobody even attempts a steal).
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(4),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    let pollers = spawn_busy_pollers(&scheduler, 12);

    wait_until(10_000, || {
        pollers
            .iter()
            .any(|(_, _, slices)| slices.load(Ordering::Acquire) > 100)
    });

    for (pid, _, _) in &pollers {
        assert!(
            scheduler.enqueue_atom_message(*pid, Atom::OK),
            "enqueue to live busy poller must succeed"
        );
    }
    wait_until(2_000, || all_observed_atom(&pollers, Atom::OK));
    scheduler.shutdown();
}

// ---------------------------------------------------------------------------
// Starvation under spawn/exit churn (production shape): a wave of busy-poll
// natives exits while a new wave spawns through the SUPERVISED native spawn
// path (SchedulerSpawnFacility::spawn_native → wait_set.woken tagged for
// scheduler thread 0), the exact shape of liminal-server's connection
// scheduler under a worker restart. The 2026-07 production incident showed
// exactly thread_count natives beating while every other live native received
// ZERO slices until a beater exited — the LIFO owner-pop signature: the
// just-requeued spinner is re-popped immediately, one spinner per thread
// monopolizes its stack top, everything below is never popped, wakes no-op
// (runnable pids are not in the wait set), and no thread ever steals because
// no thread's own queue ever empties. These tests fail within one churn
// cycle if the owner pop ever regresses to LIFO (verified by flipping
// `Worker::new_fifo` to `new_lifo`), and additionally pin the supervised
// spawn path + concurrent exit/spawn/message-delivery interleavings that the
// co-resident tests above do not exercise.
//
// NOTE: the production starvation observed on 2026-07-06/07 with these
// symptoms was NOT a bug on this main — it was a duplicate-dependency build:
// the aion server's [patch] pointed beamr at the fixed local checkout for
// aion's own `beamr ^0.12` dep, while liminal-server's `beamr = "0.11.0"`
// (workspace) requirement silently resolved to crates.io beamr 0.11.0, whose
// RunQueue is LIFO (as is published 0.12.0 — the FIFO fix has not been
// released). The connection scheduler is constructed inside liminal-server,
// so production ran the unfixed copy.
// ---------------------------------------------------------------------------

struct ChurnBusyPollHandler {
    received: Arc<Mutex<Vec<Term>>>,
    slices: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl crate::native::native_process::NativeHandler for ChurnBusyPollHandler {
    fn handle(
        &mut self,
        ctx: &mut crate::native::native_process::NativeContext<'_>,
    ) -> crate::native::native_process::NativeOutcome {
        self.slices.fetch_add(1, Ordering::AcqRel);
        while let Some(message) = ctx.recv() {
            lock_or_recover(&self.received).push(message);
        }
        if self.stop.load(Ordering::Acquire) {
            crate::native::native_process::NativeOutcome::Stop(ExitReason::Normal)
        } else {
            crate::native::native_process::NativeOutcome::Continue
        }
    }
}

struct ChurnPoller {
    pid: u64,
    received: Arc<Mutex<Vec<Term>>>,
    slices: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

fn spawn_churn_poller(scheduler: &Scheduler) -> ChurnPoller {
    let received = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let factory_received = Arc::clone(&received);
    let factory_slices = Arc::clone(&slices);
    let factory_stop = Arc::clone(&stop);
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ChurnBusyPollHandler {
                received: Arc::clone(&factory_received),
                slices: Arc::clone(&factory_slices),
                stop: Arc::clone(&factory_stop),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn native churn poller: {error:?}"));
    ChurnPoller {
        pid,
        received,
        slices,
        stop,
    }
}

#[test]
fn busy_poll_natives_all_progress_under_spawn_exit_churn() {
    // 4 scheduler threads, waves of 8 spinners. Each cycle: a new wave spawns
    // interleaved with the old wave being told to exit (worker-restart shape).
    // After each cycle EVERY live spinner must keep accruing slices, and an
    // atom enqueued to each must be observed by its handler.
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(4),
            dirty_cpu_threads: Some(1),
            dirty_io_threads: Some(1),
            dirty_queue_depth: Some(8),
            ..SchedulerConfig::default()
        },
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));

    let mut wave: Vec<ChurnPoller> = (0..8).map(|_| spawn_churn_poller(&scheduler)).collect();
    wait_until(10_000, || {
        wave.iter()
            .all(|poller| poller.slices.load(Ordering::Acquire) > 100)
    });

    for cycle in 0..5 {
        // Concurrent churn: spawn one new spinner, retire one old spinner.
        let next: Vec<ChurnPoller> = (0..8)
            .map(|i| {
                let fresh = spawn_churn_poller(&scheduler);
                wave[i].stop.store(true, Ordering::Release);
                fresh
            })
            .collect();
        wait_until(10_000, || {
            wave.iter()
                .all(|poller| scheduler.peek_exit_reason(poller.pid).is_some())
        });
        wave = next;

        // Every live spinner must accrue slices over a generous window.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let before: Vec<usize> = wave
            .iter()
            .map(|poller| poller.slices.load(Ordering::Acquire))
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let after: Vec<usize> = wave
            .iter()
            .map(|poller| poller.slices.load(Ordering::Acquire))
            .collect();
        let starved: Vec<(usize, u64, usize)> = wave
            .iter()
            .enumerate()
            .filter(|(i, _)| after[*i] == before[*i])
            .map(|(i, poller)| (i, poller.pid, after[i]))
            .collect();
        assert!(
            starved.is_empty(),
            "cycle {cycle}: starved spinners (index, pid, total slices): {starved:?}; \
             all counts before={before:?} after={after:?}"
        );

        // And a message to each must be observed.
        for poller in &wave {
            assert!(
                scheduler.enqueue_atom_message(poller.pid, Atom::OK),
                "enqueue to live busy poller must succeed"
            );
        }
        wait_until(5_000, || {
            wave.iter()
                .all(|poller| lock_or_recover(&poller.received).contains(&Term::atom(Atom::OK)))
        });
    }
    scheduler.shutdown();
}

#[test]
fn busy_poll_natives_all_progress_under_heavy_concurrent_churn() {
    // Harder production mirror: an embedder thread keeps spawning replacement
    // spinners while old ones exit mid-slice, with concurrent message load.
    // Repeated many cycles; every live spinner must keep accruing slices.
    let scheduler = Arc::new(
        Scheduler::new(
            SchedulerConfig {
                thread_count: Some(4),
                dirty_cpu_threads: Some(1),
                dirty_io_threads: Some(1),
                dirty_queue_depth: Some(8),
                ..SchedulerConfig::default()
            },
            Arc::new(ModuleRegistry::new()),
        )
        .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    );

    let mut wave: Vec<ChurnPoller> = (0..9).map(|_| spawn_churn_poller(&scheduler)).collect();
    wait_until(10_000, || {
        wave.iter()
            .all(|poller| poller.slices.load(Ordering::Acquire) > 100)
    });

    for cycle in 0..20 {
        // Message blaster: keeps delivering atoms to the current wave from a
        // second embedder thread while churn happens.
        let blast_pids: Vec<u64> = wave.iter().map(|poller| poller.pid).collect();
        let blast_scheduler = Arc::clone(&scheduler);
        let blast_stop = Arc::new(AtomicBool::new(false));
        let blast_stop_thread = Arc::clone(&blast_stop);
        let blaster = std::thread::spawn(move || {
            while !blast_stop_thread.load(Ordering::Acquire) {
                for pid in &blast_pids {
                    let _ = blast_scheduler.enqueue_atom_message(*pid, Atom::OK);
                }
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        });

        // Churn: replace the whole wave, one-in-one-out with jitter.
        let next: Vec<ChurnPoller> = (0..9)
            .map(|i| {
                let fresh = spawn_churn_poller(&scheduler);
                wave[i].stop.store(true, Ordering::Release);
                if i % 3 == 0 {
                    std::thread::sleep(std::time::Duration::from_micros(50 * (i as u64 + 1)));
                }
                fresh
            })
            .collect();
        wait_until(10_000, || {
            wave.iter()
                .all(|poller| scheduler.peek_exit_reason(poller.pid).is_some())
        });
        blast_stop.store(true, Ordering::Release);
        blaster
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
        wave = next;

        std::thread::sleep(std::time::Duration::from_millis(100));
        let before: Vec<usize> = wave
            .iter()
            .map(|poller| poller.slices.load(Ordering::Acquire))
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let after: Vec<usize> = wave
            .iter()
            .map(|poller| poller.slices.load(Ordering::Acquire))
            .collect();
        let starved: Vec<(usize, u64, usize)> = wave
            .iter()
            .enumerate()
            .filter(|(i, _)| after[*i] == before[*i])
            .map(|(i, poller)| (i, poller.pid, after[i]))
            .collect();
        assert!(
            starved.is_empty(),
            "cycle {cycle}: starved spinners (index, pid, total slices): {starved:?}; \
             all counts before={before:?} after={after:?}"
        );
    }
    scheduler.shutdown();
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

// ── Commit-5 round-1 pins ───────────────────────────────────────────────────

/// Round-1 major 2 (group leader): a top-level process answers to process 0 —
/// the standard-IO server — when the standard ring is Owned, and to the
/// no-such-pid sentinel when it is Disabled, so `io:*` takes the
/// `{error,noproc}` send-failure arm instead of self-queueing an io_request
/// and parking forever.
static GL_PROBE_SEEN: AtomicUsize = AtomicUsize::new(0);
static GL_PROBE_LEADER: AtomicU64 = AtomicU64::new(0);

fn gl_probe_native(
    _args: &[Term],
    context: &mut crate::native::ProcessContext,
) -> Result<Term, Term> {
    // Report the attached process's OWN group leader from inside the native —
    // the body slot is Executing while this runs, so an outside probe cannot
    // read it, but the context can.
    let leader = context
        .group_leader()
        .ok()
        .and_then(|term| term.as_pid())
        .unwrap_or(u64::MAX - 1);
    GL_PROBE_LEADER.store(leader, Ordering::Release);
    GL_PROBE_SEEN.fetch_add(1, Ordering::AcqRel);
    Ok(Term::atom(Atom::OK))
}

/// Round-1 major 2 (group leader): a top-level process answers to process 0 —
/// the standard-IO server — when the standard ring is Owned, and to the
/// no-such-pid sentinel when it is Disabled, so `io:*` takes the
/// `{error,noproc}` send-failure arm instead of self-queueing an io_request
/// and parking forever.
#[test]
fn top_level_group_leader_is_process_zero_when_owned_and_sentinel_when_disabled() {
    let registry = Arc::new(ModuleRegistry::new());
    let atoms = AtomTable::new();
    let name = atoms.intern("gl_probe");
    let module = native_call_module(&registry, name, gl_probe_native, None);
    GL_PROBE_SEEN.store(0, Ordering::Release);

    // Legacy/default profile: standard ring Owned, process 0 registered.
    let owned = single_thread_scheduler(&registry);
    let _owned_pid = owned.spawn_process(&module);
    wait_until(10_000, || GL_PROBE_SEEN.load(Ordering::Acquire) == 1);
    assert_eq!(
        GL_PROBE_LEADER.load(Ordering::Acquire),
        0,
        "top-level processes answer to process 0 when the standard ring is Owned"
    );
    // A message SEND to the group leader genuinely lands (process 0 exists
    // and has a mailbox; enqueue keys on the same process_bodies lookup the
    // io-message facility uses) — the positive control for the noproc pin.
    assert!(
        owned.enqueue_atom_message(0, Atom::OK),
        "a message to process 0 is deliverable on the Owned profile"
    );
    owned.shutdown();

    // minimal(): no standard ring, no process 0 — the leader is the sentinel,
    // and a send to it FAILS, which is exactly the io_bifs noproc arm.
    let minimal = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal(),
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("minimal scheduler starts: {error}"));
    let _minimal_pid = minimal.spawn_process(&module);
    wait_until(10_000, || GL_PROBE_SEEN.load(Ordering::Acquire) == 2);
    assert_eq!(
        GL_PROBE_LEADER.load(Ordering::Acquire),
        NO_STANDARD_IO_PID,
        "with the standard ring Disabled the leader is the no-such-pid sentinel"
    );
    assert!(
        !minimal.enqueue_atom_message(NO_STANDARD_IO_PID, Atom::OK),
        "a send to the sentinel leader FAILS — io:* takes the {{error,noproc}} \
         arm before any suspension instead of parking forever"
    );
    minimal.shutdown();
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
    )
    .unwrap_or_else(|error| panic!("scheduler A starts: {error}"));
    let scheduler_b = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal().shared_dirty_cpu(Arc::clone(&pool)),
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("scheduler B starts: {error}"));

    // A's dirty job enters the shared pool and BLOCKS there.
    let blocked_pid = scheduler_a.spawn_process(&blocking_module);
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
    let b_pid = scheduler_b.spawn_process(&quick_module);
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

/// Round-2 minor: the public `spawn_native` root (synthetic caller 0) is
/// seeded from `standard_io_pid` exactly like a top-level bytecode spawn —
/// process 0 when Owned, the no-such-pid sentinel when Disabled — never the
/// absent caller's own pid.
#[test]
fn spawn_native_root_leader_matches_the_standard_io_pid() {
    let registry = Arc::new(ModuleRegistry::new());

    let owned = single_thread_scheduler(&registry);
    let received = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let (factory_received, factory_slices) = (Arc::clone(&received), Arc::clone(&slices));
    let owned_pid = owned
        .spawn_native(Box::new(move || {
            Box::new(BusyPollHandler {
                received: Arc::clone(&factory_received),
                slices: Arc::clone(&factory_slices),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn native: {error:?}"));
    wait_until(10_000, || {
        owned.test_group_leader(owned_pid) == Some(Term::pid(0))
    });
    owned.shutdown();

    let minimal = Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        SchedulerServices::minimal(),
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("minimal scheduler starts: {error}"));
    let received = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let (factory_received, factory_slices) = (Arc::clone(&received), Arc::clone(&slices));
    let minimal_pid = minimal
        .spawn_native(Box::new(move || {
            Box::new(BusyPollHandler {
                received: Arc::clone(&factory_received),
                slices: Arc::clone(&factory_slices),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn native: {error:?}"));
    wait_until(10_000, || {
        minimal.test_group_leader(minimal_pid) == Some(Term::pid(NO_STANDARD_IO_PID))
    });
    minimal.shutdown();
}

/// Round-3 major: EVERY `SpawnFacility` method refuses after teardown — not
/// just `spawn`. A dirty native surviving on an embedder-owned shared pool
/// can reach any of them through its retained context.
#[test]
fn every_spawn_facility_method_refuses_after_teardown() {
    use crate::native::SpawnFacility as _;
    use crate::native::spawn::SpawnError;

    let atoms = AtomTable::new();
    let module_name = atoms.intern("teardown_refusal");
    let function = atoms.intern("main");
    let mut module = test_module(module_name, vec![Instruction::Label { label: 7 }]);
    module.exports.insert((function, 0), 7);
    let registry = Arc::new(ModuleRegistry::new());
    let _module = registry.insert(module);
    let scheduler = Scheduler::new(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        Arc::clone(&registry),
    )
    .unwrap_or_else(|error| panic!("scheduler starts: {error}"));
    scheduler.shutdown();

    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };
    let processes_before = scheduler.shared.process_bodies.len();

    assert_eq!(
        facility.spawn(1, module_name, function, Vec::new(), None),
        Err(SpawnError::SchedulerTearingDown)
    );
    let received = Arc::new(Mutex::new(Vec::new()));
    let slices = Arc::new(AtomicUsize::new(0));
    let (factory_received, factory_slices) = (Arc::clone(&received), Arc::clone(&slices));
    assert_eq!(
        facility.spawn_native(
            1,
            Box::new(move || {
                Box::new(BusyPollHandler {
                    received: Arc::clone(&factory_received),
                    slices: Arc::clone(&factory_slices),
                })
            }),
            None
        ),
        Err(SpawnError::SchedulerTearingDown)
    );
    assert!(matches!(
        facility.spawn_monitor(1, module_name, function, Vec::new()),
        Err(SpawnError::SchedulerTearingDown)
    ));
    assert!(matches!(
        facility.spawn_lambda(1, module_name, 0, None),
        Err(SpawnError::SchedulerTearingDown)
    ));
    assert!(matches!(
        facility.spawn_lambda_monitor(1, module_name, 0),
        Err(SpawnError::SchedulerTearingDown)
    ));
    assert!(matches!(
        facility.spawn_with_options(
            1,
            module_name,
            function,
            Vec::new(),
            SpawnOptions::default()
        ),
        Err(SpawnError::SchedulerTearingDown)
    ));
    assert!(matches!(
        facility.spawn_lambda_with_options(1, module_name, 0, SpawnOptions::default()),
        Err(SpawnError::SchedulerTearingDown)
    ));

    assert_eq!(
        scheduler.shared.process_bodies.len(),
        processes_before,
        "no refused spawn path touched process state"
    );
}

/// Round-4 major: spawn admission is LINEARIZED with teardown, not a
/// snapshot. A spawn HELD immediately after admission (test barrier) forces
/// the interleaving Sol found: shutdown must WAIT for the admitted spawn (its
/// mutation lands before shutdown returns), and the next spawn refuses. In no
/// case does process state change after shutdown has returned.
#[test]
fn shutdown_waits_for_an_admitted_spawn_and_refuses_the_next() {
    use crate::native::SpawnFacility as _;
    use crate::native::spawn::SpawnError;

    let atoms = AtomTable::new();
    let module_name = atoms.intern("admitted_spawn");
    let function = atoms.intern("main");
    let mut module = test_module(
        module_name,
        vec![
            Instruction::Label { label: 7 },
            Instruction::Wait {
                fail: Operand::Label(7),
            },
        ],
    );
    module.exports.insert((function, 0), 7);
    let registry = Arc::new(ModuleRegistry::new());
    let _module = registry.insert(module);
    let scheduler = Arc::new(
        Scheduler::new(
            SchedulerConfig {
                thread_count: Some(1),
                ..SchedulerConfig::default()
            },
            Arc::clone(&registry),
        )
        .unwrap_or_else(|error| panic!("scheduler starts: {error}")),
    );

    // RAII reset: the hold target is cleared even if an assertion below
    // unwinds, so a failure here cannot park other tests' spawns.
    struct HoldReset;
    impl Drop for HoldReset {
        fn drop(&mut self) {
            supervision_integration::SPAWN_HOLD_TARGET.store(0, Ordering::Release);
        }
    }
    let _hold_reset = HoldReset;
    supervision_integration::SPAWN_HELD_AT_GATE.store(false, Ordering::Release);
    supervision_integration::SPAWN_HOLD_TARGET
        .store(Arc::as_ptr(&scheduler.shared) as usize, Ordering::Release);

    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };
    let spawn_thread =
        std::thread::spawn(move || facility.spawn(1, module_name, function, Vec::new(), None));
    wait_until(10_000, || {
        supervision_integration::SPAWN_HELD_AT_GATE.load(Ordering::Acquire)
    });

    // Shutdown starts while the ADMITTED spawn is held at the barrier: the
    // drain must wait for it, not return around it.
    let shutdown_done = Arc::new(AtomicBool::new(false));
    let (shutdown_scheduler, shutdown_flag) = (Arc::clone(&scheduler), Arc::clone(&shutdown_done));
    let shutdown_thread = std::thread::spawn(move || {
        shutdown_scheduler.shutdown();
        shutdown_flag.store(true, Ordering::Release);
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !shutdown_done.load(Ordering::Acquire),
        "shutdown must WAIT for the admitted spawn, not return around it"
    );

    // Release the admitted spawn: it completes (its mutation lands BEFORE
    // shutdown returns), then shutdown finishes.
    supervision_integration::SPAWN_HOLD_TARGET.store(0, Ordering::Release);
    let spawned = spawn_thread
        .join()
        .unwrap_or_else(|_| panic!("spawn thread joins"))
        .unwrap_or_else(|error| panic!("the admitted spawn completes: {error:?}"));
    shutdown_thread
        .join()
        .unwrap_or_else(|_| panic!("shutdown thread joins"));
    assert!(shutdown_done.load(Ordering::Acquire));
    assert!(
        scheduler.shared.process_bodies.contains_key(&spawned),
        "the admitted spawn's process exists — its mutation preceded shutdown's return"
    );

    // And the NEXT spawn refuses: intake is closed.
    let facility = supervision_integration::SchedulerSpawnFacility {
        shared: Arc::clone(&scheduler.shared),
        namespace_id: NamespaceId::DEFAULT,
    };
    assert_eq!(
        facility.spawn(1, module_name, function, Vec::new(), None),
        Err(SpawnError::SchedulerTearingDown)
    );
}
