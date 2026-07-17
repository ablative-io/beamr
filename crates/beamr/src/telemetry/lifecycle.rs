//! OpenTelemetry log helpers for process lifecycle events.

use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider, Severity};
use opentelemetry::trace::TraceContextExt;
use opentelemetry::{Context, InstrumentationScope, Key};

use crate::atom::{Atom, AtomTable};
use crate::process::{Exception, ExitReason};
use crate::term::format::format_term;

const LOGGER_NAME: &str = "beamr";
const EVENT_PROCESS_SPAWNED: &str = "process.spawned";
const EVENT_PROCESS_EXITED: &str = "process.exited";
const EVENT_PROCESS_LINKED: &str = "process.linked";
const EVENT_PROCESS_MONITORED: &str = "process.monitored";
const EVENT_PROCESS_CRASHED: &str = "process.crashed";

static LIFECYCLE_EMITTER: RwLock<Option<Arc<dyn LifecycleLogEmitter>>> = RwLock::new(None);

trait LifecycleLogEmitter: Send + Sync {
    fn emit(&self, event_name: &'static str, severity: Severity, attributes: Vec<(Key, AnyValue)>);
}

struct ProviderLifecycleLogEmitter<P> {
    provider: P,
}

impl<P> LifecycleLogEmitter for ProviderLifecycleLogEmitter<P>
where
    P: LoggerProvider + Send + Sync,
    P::Logger: Send + Sync,
{
    fn emit(&self, event_name: &'static str, severity: Severity, attributes: Vec<(Key, AnyValue)>) {
        let logger = self.provider.logger_with_scope(lifecycle_scope());
        if !logger.event_enabled(severity, LOGGER_NAME, Some(event_name)) {
            return;
        }
        let mut record = logger.create_log_record();
        let now = SystemTime::now();
        record.set_event_name(event_name);
        record.set_target(LOGGER_NAME);
        record.set_timestamp(now);
        record.set_observed_timestamp(now);
        record.set_severity_number(severity);
        record.set_severity_text(severity.name());
        attach_current_trace_context(&mut record);
        record.add_attributes(attributes);
        logger.emit(record);
    }
}

/// Install the OpenTelemetry logger provider used for Beamr lifecycle events.
///
/// Without an installed provider lifecycle helpers are no-ops, preserving the
/// optional telemetry feature's zero-configuration behavior.
pub fn set_lifecycle_logger_provider<P>(provider: P)
where
    P: LoggerProvider + Send + Sync + 'static,
    P::Logger: Send + Sync,
{
    let mut guard = write_emitter_slot();
    *guard = Some(Arc::new(ProviderLifecycleLogEmitter { provider }));
}

// Each `record_*` helper is a thin wrapper over a `record_*_with_emitter`
// form that takes its emitter explicitly. Production call sites go through
// the wrappers (and therefore the process-global slot); tests can drive the
// identical record-construction path against an instance-scoped emitter that
// concurrent global emission can never reach.

pub(crate) fn record_process_spawned(
    atom_table: &AtomTable,
    pid: u64,
    parent_pid: u64,
    module: Atom,
    function: Atom,
    arity: u8,
) {
    if let Some(emitter) = current_emitter() {
        record_process_spawned_with_emitter(
            emitter.as_ref(),
            atom_table,
            pid,
            parent_pid,
            module,
            function,
            arity,
        );
    }
}

fn record_process_spawned_with_emitter(
    emitter: &dyn LifecycleLogEmitter,
    atom_table: &AtomTable,
    pid: u64,
    parent_pid: u64,
    module: Atom,
    function: Atom,
    arity: u8,
) {
    emitter.emit(
        EVENT_PROCESS_SPAWNED,
        Severity::Info,
        vec![
            int_attr("process.pid", pid_to_i64(pid)),
            int_attr("pid", pid_to_i64(pid)),
            int_attr("process.parent_pid", pid_to_i64(parent_pid)),
            int_attr("parent_pid", pid_to_i64(parent_pid)),
            string_attr("code.module", atom_name(atom_table, module)),
            string_attr("module", atom_name(atom_table, module)),
            string_attr("code.function", atom_name(atom_table, function)),
            string_attr("function", atom_name(atom_table, function)),
            int_attr("code.arity", i64::from(arity)),
            int_attr("arity", i64::from(arity)),
        ],
    );
}

pub(crate) fn record_process_exited(atom_table: &AtomTable, pid: u64, reason: ExitReason) {
    if let Some(emitter) = current_emitter() {
        record_process_exited_with_emitter(emitter.as_ref(), atom_table, pid, reason);
    }
}

fn record_process_exited_with_emitter(
    emitter: &dyn LifecycleLogEmitter,
    atom_table: &AtomTable,
    pid: u64,
    reason: ExitReason,
) {
    emitter.emit(
        EVENT_PROCESS_EXITED,
        Severity::Info,
        vec![
            int_attr("process.pid", pid_to_i64(pid)),
            int_attr("pid", pid_to_i64(pid)),
            string_attr(
                "process.exit.reason",
                atom_name(atom_table, reason.as_atom()),
            ),
            string_attr("reason", atom_name(atom_table, reason.as_atom())),
            string_attr("process.exit_class", exit_class(reason)),
            string_attr("exit_class", exit_class(reason)),
        ],
    );
}

pub(crate) fn record_process_linked(pid_a: u64, pid_b: u64) {
    if let Some(emitter) = current_emitter() {
        record_process_linked_with_emitter(emitter.as_ref(), pid_a, pid_b);
    }
}

fn record_process_linked_with_emitter(emitter: &dyn LifecycleLogEmitter, pid_a: u64, pid_b: u64) {
    emitter.emit(
        EVENT_PROCESS_LINKED,
        Severity::Info,
        vec![
            int_attr("process.pid_a", pid_to_i64(pid_a)),
            int_attr("pid_a", pid_to_i64(pid_a)),
            int_attr("process.pid_b", pid_to_i64(pid_b)),
            int_attr("pid_b", pid_to_i64(pid_b)),
        ],
    );
}

pub(crate) fn record_process_monitored(watcher_pid: u64, target_pid: u64, reference: u64) {
    if let Some(emitter) = current_emitter() {
        record_process_monitored_with_emitter(emitter.as_ref(), watcher_pid, target_pid, reference);
    }
}

fn record_process_monitored_with_emitter(
    emitter: &dyn LifecycleLogEmitter,
    watcher_pid: u64,
    target_pid: u64,
    reference: u64,
) {
    emitter.emit(
        EVENT_PROCESS_MONITORED,
        Severity::Info,
        vec![
            int_attr("process.watcher_pid", pid_to_i64(watcher_pid)),
            int_attr("watcher_pid", pid_to_i64(watcher_pid)),
            int_attr("process.target_pid", pid_to_i64(target_pid)),
            int_attr("target_pid", pid_to_i64(target_pid)),
            int_attr("process.monitor.ref", pid_to_i64(reference)),
            int_attr("ref", pid_to_i64(reference)),
        ],
    );
}

pub(crate) fn record_process_crashed(atom_table: &AtomTable, pid: u64, exception: Exception) {
    if let Some(emitter) = current_emitter() {
        record_process_crashed_with_emitter(emitter.as_ref(), atom_table, pid, exception);
    }
}

fn record_process_crashed_with_emitter(
    emitter: &dyn LifecycleLogEmitter,
    atom_table: &AtomTable,
    pid: u64,
    exception: Exception,
) {
    emitter.emit(
        EVENT_PROCESS_CRASHED,
        Severity::Error,
        vec![
            int_attr("process.pid", pid_to_i64(pid)),
            int_attr("pid", pid_to_i64(pid)),
            string_attr("exception.class", format_term(exception.class, atom_table)),
            string_attr("exception_class", format_term(exception.class, atom_table)),
            string_attr(
                "exception.reason",
                format_term(exception.reason, atom_table),
            ),
            string_attr(
                "exception.stacktrace",
                format_term(exception.stacktrace, atom_table),
            ),
            string_attr("reason", exception.format_with_atoms(atom_table)),
            string_attr("stacktrace", format_term(exception.stacktrace, atom_table)),
        ],
    );
}

pub(crate) fn record_process_crashed_reason(atom_table: &AtomTable, pid: u64, reason: ExitReason) {
    if let Some(emitter) = current_emitter() {
        record_process_crashed_reason_with_emitter(emitter.as_ref(), atom_table, pid, reason);
    }
}

fn record_process_crashed_reason_with_emitter(
    emitter: &dyn LifecycleLogEmitter,
    atom_table: &AtomTable,
    pid: u64,
    reason: ExitReason,
) {
    let reason_name = atom_name(atom_table, reason.as_atom());
    emitter.emit(
        EVENT_PROCESS_CRASHED,
        Severity::Error,
        vec![
            int_attr("process.pid", pid_to_i64(pid)),
            int_attr("pid", pid_to_i64(pid)),
            string_attr("exception.class", "error"),
            string_attr("exception_class", "error"),
            string_attr("exception.reason", reason_name.clone()),
            string_attr("exception.stacktrace", "[]"),
            string_attr("reason", reason_name),
            string_attr("stacktrace", "[]"),
        ],
    );
}

/// Clone the currently installed process-global emitter, if any, so emission
/// happens outside the slot lock.
fn current_emitter() -> Option<Arc<dyn LifecycleLogEmitter>> {
    let guard = read_emitter_slot();
    guard.as_ref().map(Arc::clone)
}

fn lifecycle_scope() -> InstrumentationScope {
    InstrumentationScope::builder(LOGGER_NAME)
        .with_version(env!("CARGO_PKG_VERSION"))
        .build()
}

fn attach_current_trace_context<R>(record: &mut R)
where
    R: LogRecord,
{
    let context = Context::current();
    let span_context = context.span().span_context().clone();
    if span_context.is_valid() {
        record.set_trace_context(
            span_context.trace_id(),
            span_context.span_id(),
            Some(span_context.trace_flags()),
        );
    }
}

fn read_emitter_slot() -> std::sync::RwLockReadGuard<'static, Option<Arc<dyn LifecycleLogEmitter>>>
{
    match LIFECYCLE_EMITTER.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_emitter_slot() -> std::sync::RwLockWriteGuard<'static, Option<Arc<dyn LifecycleLogEmitter>>>
{
    match LIFECYCLE_EMITTER.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn int_attr(key: &'static str, value: i64) -> (Key, AnyValue) {
    (Key::from_static_str(key), AnyValue::Int(value))
}

fn string_attr(key: &'static str, value: impl Into<opentelemetry::StringValue>) -> (Key, AnyValue) {
    (Key::from_static_str(key), AnyValue::String(value.into()))
}

fn pid_to_i64(pid: u64) -> i64 {
    i64::try_from(pid).unwrap_or(i64::MAX)
}

fn atom_name(atom_table: &AtomTable, atom: Atom) -> String {
    atom_table
        .resolve(atom)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("atom:{}", atom.index()))
}

const fn exit_class(reason: ExitReason) -> &'static str {
    match reason {
        ExitReason::Normal => "normal",
        ExitReason::Kill
        | ExitReason::Killed
        | ExitReason::Error
        | ExitReason::NoConnection
        | ExitReason::NoProc => "error",
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
