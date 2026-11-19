//! Telemetry metrics: VM health and per-process gauges a slice records.

use super::*;

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

    // Register the process in the scheduler's table so the alive-process
    // gauge (`process_count() == process_table.len()`) reflects a real live
    // process rather than zero.
    scheduler.shared.process_table.spawn_with_pid(55);
    let mut process = Process::new(55, DEFAULT_HEAP_SIZE);
    process.set_code_position(Some(CodePosition {
        module: module_name,
        // Enter at the label AFTER `func_info`: that opcode is the
        // clause-dispatch landing pad and raises `function_clause`
        // (5bcd529), so ip 0 kills the process before the slice runs.
        // `Scheduler::spawn_in` resolves `label_ip` for the same reason.
        instruction_pointer: 1,
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

    scheduler.shared.process_bodies.insert(
        55,
        Mutex::new(ProcessSlot::Present(ScheduledProcess(process))),
    );
    // A direct slice bypasses the worker loop's execution-health sample.
    scheduler
        .shared
        .record_scheduler_executing(std::time::Duration::ZERO);

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

    scheduler.shutdown();
    assert_eq!(scheduler.shared.process_count(), 0);
    provider.force_flush().expect("metrics flush after drain");
    let drained_metrics = exporter
        .get_finished_metrics()
        .expect("finished drain metrics");
    let latest = drained_metrics.last().expect("drain sample exists");
    let metric = find_metric(std::slice::from_ref(latest), "beamr.processes.alive")
        .expect("drain sample contains live gauge");
    let AggregatedMetrics::U64(MetricData::Gauge(gauge)) = metric.data() else {
        panic!("live-process metric must be an unsigned gauge");
    };
    let mut points = gauge.data_points();
    assert_eq!(
        points
            .next()
            .map(opentelemetry_sdk::metrics::data::GaugeDataPoint::value),
        Some(0)
    );
    assert!(points.all(|point| point.value() == 0));

    provider.shutdown().expect("provider shutdown");
}
