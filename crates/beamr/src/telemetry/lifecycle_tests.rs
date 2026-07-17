use super::*;
use opentelemetry::logs::Severity;
use opentelemetry::{Key, logs::AnyValue};
use opentelemetry_sdk::logs::in_memory_exporter::LogDataWithResource;
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};

/// Instance-scoped lifecycle recorder: events recorded through it flow
/// through the same `record_*_with_emitter` construction path as the global
/// helpers, but into an emitter this value owns. Concurrent tests emitting
/// through the process-global slot can never reach its exporter, so
/// attribute assertions against it are immune to foreign lifecycle events.
struct LifecycleRecorder {
    exporter: InMemoryLogExporter,
    provider: SdkLoggerProvider,
    emitter: ProviderLifecycleLogEmitter<SdkLoggerProvider>,
}

impl LifecycleRecorder {
    fn new() -> Self {
        let exporter = InMemoryLogExporter::default();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let emitter = ProviderLifecycleLogEmitter {
            provider: provider.clone(),
        };
        Self {
            exporter,
            provider,
            emitter,
        }
    }

    fn record_process_spawned(
        &self,
        atom_table: &AtomTable,
        pid: u64,
        parent_pid: u64,
        module: Atom,
        function: Atom,
        arity: u8,
    ) {
        record_process_spawned_with_emitter(
            &self.emitter,
            atom_table,
            pid,
            parent_pid,
            module,
            function,
            arity,
        );
    }

    fn record_process_exited(&self, atom_table: &AtomTable, pid: u64, reason: ExitReason) {
        record_process_exited_with_emitter(&self.emitter, atom_table, pid, reason);
    }

    fn record_process_linked(&self, pid_a: u64, pid_b: u64) {
        record_process_linked_with_emitter(&self.emitter, pid_a, pid_b);
    }

    fn record_process_monitored(&self, watcher_pid: u64, target_pid: u64, reference: u64) {
        record_process_monitored_with_emitter(&self.emitter, watcher_pid, target_pid, reference);
    }

    fn record_process_crashed(&self, atom_table: &AtomTable, pid: u64, exception: Exception) {
        record_process_crashed_with_emitter(&self.emitter, atom_table, pid, exception);
    }

    fn flushed_logs(&self) -> Vec<LogDataWithResource> {
        self.provider.force_flush().expect("logs flush");
        self.exporter.get_emitted_logs().expect("emitted logs")
    }

    fn shutdown(&self) {
        self.provider.shutdown().expect("provider shutdown");
    }
}

fn install_test_provider() -> (InMemoryLogExporter, SdkLoggerProvider) {
    let exporter = InMemoryLogExporter::default();
    let provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    set_lifecycle_logger_provider(provider.clone());
    (exporter, provider)
}

fn attr_i64(log: &opentelemetry_sdk::logs::SdkLogRecord, key: &str) -> Option<i64> {
    log.attributes_iter().find_map(|(attribute_key, value)| {
        (attribute_key == &Key::new(key.to_owned())).then_some(match value {
            AnyValue::Int(value) => Some(*value),
            _ => None,
        })?
    })
}

fn attr_string(log: &opentelemetry_sdk::logs::SdkLogRecord, key: &str) -> Option<String> {
    log.attributes_iter().find_map(|(attribute_key, value)| {
        (attribute_key == &Key::new(key.to_owned())).then(|| match value {
            AnyValue::String(value) => Some(value.to_string()),
            _ => None,
        })?
    })
}

#[test]
fn lifecycle_helpers_emit_process_events_with_attributes() {
    let recorder = LifecycleRecorder::new();
    let atom_table = AtomTable::with_common_atoms();
    let module = atom_table.intern("demo_module");
    let function = atom_table.intern("start");

    recorder.record_process_spawned(&atom_table, 2, 1, module, function, 3);
    recorder.record_process_linked(1, 2);
    recorder.record_process_monitored(1, 2, 99);
    recorder.record_process_exited(&atom_table, 2, ExitReason::Normal);

    let logs = recorder.flushed_logs();
    let spawned = logs
        .iter()
        .find(|log| log.record.event_name() == Some(EVENT_PROCESS_SPAWNED))
        .expect("spawned event emitted");
    assert_eq!(
        spawned.record.target().map(|target| target.as_ref()),
        Some(LOGGER_NAME)
    );
    assert!(spawned.record.timestamp().is_some());
    assert!(spawned.record.observed_timestamp().is_some());
    assert_eq!(spawned.record.severity_number(), Some(Severity::Info));
    assert_eq!(attr_i64(&spawned.record, "process.pid"), Some(2));
    assert_eq!(attr_i64(&spawned.record, "parent_pid"), Some(1));
    assert_eq!(
        attr_string(&spawned.record, "module"),
        Some("demo_module".to_owned())
    );
    assert_eq!(
        attr_string(&spawned.record, "function"),
        Some("start".to_owned())
    );
    assert_eq!(attr_i64(&spawned.record, "arity"), Some(3));

    let linked = logs
        .iter()
        .find(|log| log.record.event_name() == Some(EVENT_PROCESS_LINKED))
        .expect("linked event emitted");
    assert_eq!(attr_i64(&linked.record, "pid_a"), Some(1));
    assert_eq!(attr_i64(&linked.record, "pid_b"), Some(2));

    let monitored = logs
        .iter()
        .find(|log| log.record.event_name() == Some(EVENT_PROCESS_MONITORED))
        .expect("monitored event emitted");
    assert_eq!(attr_i64(&monitored.record, "watcher_pid"), Some(1));
    assert_eq!(attr_i64(&monitored.record, "target_pid"), Some(2));
    assert_eq!(attr_i64(&monitored.record, "ref"), Some(99));

    let exited = logs
        .iter()
        .find(|log| log.record.event_name() == Some(EVENT_PROCESS_EXITED))
        .expect("exited event emitted");
    // Ownership first: prove the record is ours before asserting its payload,
    // so any future substitution fails at the ownership boundary.
    assert_eq!(attr_i64(&exited.record, "process.pid"), Some(2));
    assert_eq!(attr_i64(&exited.record, "pid"), Some(2));
    assert_eq!(
        attr_string(&exited.record, "reason"),
        Some("normal".to_owned())
    );
    assert_eq!(
        attr_string(&exited.record, "exit_class"),
        Some("normal".to_owned())
    );
    recorder.shutdown();
}

#[test]
fn crash_event_records_error_severity_and_exception_details() {
    let recorder = LifecycleRecorder::new();
    let atom_table = AtomTable::with_common_atoms();
    let badarg = atom_table.intern("badarg");
    let exception = Exception {
        class: crate::term::Term::atom(Atom::ERROR),
        reason: crate::term::Term::atom(badarg),
        stacktrace: crate::term::Term::NIL,
    };

    recorder.record_process_crashed(&atom_table, 42, exception);
    let logs = recorder.flushed_logs();
    let crashed = logs
        .iter()
        .find(|log| log.record.event_name() == Some(EVENT_PROCESS_CRASHED))
        .expect("crashed event emitted");
    assert_eq!(crashed.record.severity_number(), Some(Severity::Error));
    assert_eq!(attr_i64(&crashed.record, "process.pid"), Some(42));
    assert_eq!(
        attr_string(&crashed.record, "exception.class"),
        Some("error".to_owned())
    );
    assert_eq!(
        attr_string(&crashed.record, "exception_class"),
        Some("error".to_owned())
    );
    assert_eq!(
        attr_string(&crashed.record, "exception.reason"),
        Some("badarg".to_owned())
    );
    recorder.shutdown();
}

#[test]
fn supervision_tree_can_be_reconstructed_from_event_stream() {
    let recorder = LifecycleRecorder::new();
    let atom_table = AtomTable::new();
    let supervisor = atom_table.intern("supervisor");
    let worker = atom_table.intern("worker");
    let start = atom_table.intern("start_link");

    recorder.record_process_spawned(&atom_table, 10, 1, supervisor, start, 0);
    recorder.record_process_spawned(&atom_table, 11, 10, worker, start, 0);
    recorder.record_process_spawned(&atom_table, 12, 10, worker, start, 0);
    recorder.record_process_linked(10, 11);
    recorder.record_process_linked(10, 12);
    recorder.record_process_monitored(10, 12, 7);

    let logs = recorder.flushed_logs();
    let mut children_by_parent = std::collections::BTreeMap::<i64, Vec<i64>>::new();
    let mut links = Vec::new();
    let mut monitors = Vec::new();
    for log in &logs {
        match log.record.event_name() {
            Some(EVENT_PROCESS_SPAWNED) => {
                if let (Some(parent), Some(child)) = (
                    attr_i64(&log.record, "parent_pid"),
                    attr_i64(&log.record, "process.pid"),
                ) {
                    children_by_parent.entry(parent).or_default().push(child);
                }
            }
            Some(EVENT_PROCESS_LINKED) => {
                if let (Some(pid_a), Some(pid_b)) = (
                    attr_i64(&log.record, "pid_a"),
                    attr_i64(&log.record, "pid_b"),
                ) {
                    links.push((pid_a, pid_b));
                }
            }
            Some(EVENT_PROCESS_MONITORED) => {
                if let (Some(watcher), Some(target), Some(reference)) = (
                    attr_i64(&log.record, "watcher_pid"),
                    attr_i64(&log.record, "target_pid"),
                    attr_i64(&log.record, "ref"),
                ) {
                    monitors.push((watcher, target, reference));
                }
            }
            _ => {}
        }
    }

    assert_eq!(children_by_parent.get(&1), Some(&vec![10]));
    assert_eq!(children_by_parent.get(&10), Some(&vec![11, 12]));
    assert_eq!(links, vec![(10, 11), (10, 12)]);
    assert_eq!(monitors, vec![(10, 12, 7)]);
    recorder.shutdown();
}

/// The attribute-focused tests above run against instance-scoped recorders,
/// so this is the one place the global slot's wiring is proven: install a
/// provider, emit one marker event through the public helper, and require
/// that the marker reaches the installed exporter. Concurrent scheduler
/// tests may emit foreign lifecycle events into the same exporter, so this
/// selects by a marker value it controls and asserts nothing about payloads
/// foreign events could pollute.
#[test]
fn global_slot_routes_lifecycle_events_to_installed_provider() {
    // Marker pid far outside any range the scheduler tests allocate.
    const MARKER_WATCHER_PID: u64 = 0x00C0_FFEE_0000_0001;

    let _guard = crate::telemetry::test_lock::guard();
    let (exporter, provider) = install_test_provider();
    record_process_monitored(MARKER_WATCHER_PID, MARKER_WATCHER_PID + 1, 424_242);
    provider.force_flush().expect("logs flush");

    let logs = exporter.get_emitted_logs().expect("emitted logs");
    let marker_delivered = logs.iter().any(|log| {
        log.record.event_name() == Some(EVENT_PROCESS_MONITORED)
            && attr_i64(&log.record, "watcher_pid") == Some(pid_to_i64(MARKER_WATCHER_PID))
    });
    assert!(
        marker_delivered,
        "globally installed provider must receive events emitted through the global helpers"
    );
    provider.shutdown().expect("provider shutdown");
}

/// Regression wall for the parallel-run flake: while an instance-scoped
/// recorder is live, another thread finalizes Error exits through the
/// process-global path. The recorder's exporter must contain only the
/// events recorded through it — a foreign record here means the recorder
/// isolation from the global slot has regressed.
#[test]
fn instance_recorder_is_isolated_from_concurrent_global_error_exits() {
    const OWNED_PID: u64 = 7;
    const FOREIGN_PID_BASE: u64 = 9_000;
    const FOREIGN_EXITS: u64 = 256;
    const OWNED_EXITS: u64 = 32;

    let _guard = crate::telemetry::test_lock::guard();
    let recorder = LifecycleRecorder::new();
    let (_global_exporter, global_provider) = install_test_provider();
    let atom_table = AtomTable::with_common_atoms();

    let storm = std::thread::spawn(|| {
        let atom_table = AtomTable::with_common_atoms();
        for offset in 0..FOREIGN_EXITS {
            record_process_exited(&atom_table, FOREIGN_PID_BASE + offset, ExitReason::Error);
        }
    });
    for _ in 0..OWNED_EXITS {
        recorder.record_process_exited(&atom_table, OWNED_PID, ExitReason::Normal);
    }
    storm.join().expect("storm thread");

    let logs = recorder.flushed_logs();
    let exited: Vec<_> = logs
        .iter()
        .filter(|log| log.record.event_name() == Some(EVENT_PROCESS_EXITED))
        .collect();
    for log in &exited {
        assert_eq!(
            attr_i64(&log.record, "process.pid"),
            Some(pid_to_i64(OWNED_PID)),
            "foreign process.exited record leaked into the instance-scoped exporter (reason={:?})",
            attr_string(&log.record, "reason")
        );
        assert_eq!(
            attr_string(&log.record, "reason"),
            Some("normal".to_owned())
        );
    }
    assert_eq!(
        exited.len(),
        usize::try_from(OWNED_EXITS).expect("owned exit count fits usize"),
        "instance-scoped exporter must contain exactly the owned exit events"
    );
    recorder.shutdown();
    global_provider.shutdown().expect("provider shutdown");
}
