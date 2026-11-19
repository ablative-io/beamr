//! Telemetry spans a slice emits, and their nesting under a trace context.

use super::*;

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
        // Enter at the label AFTER `func_info`: that opcode is the
        // clause-dispatch landing pad and raises `function_clause`
        // (5bcd529), so ip 0 kills the process before the slice runs.
        // `Scheduler::spawn_in` resolves `label_ip` for the same reason.
        instruction_pointer: 1,
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
        // Enter at the label AFTER `func_info`: that opcode is the
        // clause-dispatch landing pad and raises `function_clause`
        // (5bcd529), so ip 0 kills the process before the slice runs.
        // `Scheduler::spawn_in` resolves `label_ip` for the same reason.
        instruction_pointer: 1,
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
        // Enter at the label AFTER `func_info`: that opcode is the
        // clause-dispatch landing pad and raises `function_clause`
        // (5bcd529), so ip 0 kills the process before the slice runs.
        // `Scheduler::spawn_in` resolves `label_ip` for the same reason.
        instruction_pointer: 1,
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
