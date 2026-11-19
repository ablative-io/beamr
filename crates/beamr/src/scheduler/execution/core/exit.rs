//! A process's exit from a slice: resolving its raise frames, and the
//! cleanup and finalisation that release what it held.

use crate::gc::release_all_refcounted_resources;
use crate::io::resource::close_owned_resource_at;
use crate::process::{ExitReason, Process, ProcessStatus};
use crate::scheduler::{
    ProcessSlot, ScheduledProcess, SharedState, lock_or_recover, supervision_integration,
};
use crate::term::boxed::BoxedTag;

use super::SliceOutcome;

pub(super) fn exit_process(
    shared: &SharedState,
    process: &mut Process,
    reason: ExitReason,
) -> SliceOutcome {
    let pid = process.pid();
    // Capture before `terminate` below replaces the heap the result points
    // into.
    let result = crate::scheduler::exit_capture::capture_term(process.x_reg(0));
    if let Some(exception) = process.current_exception() {
        #[cfg(feature = "telemetry")]
        crate::telemetry::lifecycle::record_process_crashed(&shared.atom_table, pid, exception);
        // Capture while the process heap is still alive; the exception terms
        // point into it and the heap is freed during cleanup. Raise-time raw
        // frames are resolved to names here so diagnostics keep a usable
        // stacktrace even when the exception term carries none.
        let frames = resolve_raise_frames(&shared.atom_table, process);
        shared.exit_exceptions.insert(
            pid,
            crate::scheduler::exit_capture::OwnedException::capture_with_frames(exception, frames),
        );
    } else if reason != ExitReason::Normal {
        #[cfg(feature = "telemetry")]
        crate::telemetry::lifecycle::record_process_crashed_reason(&shared.atom_table, pid, reason);
    }
    #[cfg(feature = "telemetry")]
    if let Some(trace_context) = process.trace_context() {
        trace_context.finish(exit_reason_label(reason));
    }
    process.terminate(reason);
    SliceOutcome::Exited(reason, result)
}

/// Resolve the raise-time raw stacktrace into owned, name-resolved frames.
///
/// Takes the atom table rather than the whole [`SharedState`] because name
/// resolution is all it needs, and a frame renderer that can be exercised with
/// nothing but an atom table is a frame renderer that can be tested.
fn resolve_raise_frames(
    atom_table: &crate::atom::AtomTable,
    process: &Process,
) -> Vec<crate::scheduler::exit_capture::CapturedFrame> {
    process
        .raw_stacktrace()
        .iter()
        .map(|entry| {
            let (module, function, arity) = entry.identity();
            crate::scheduler::exit_capture::CapturedFrame {
                module: atom_table
                    .resolve(module)
                    .unwrap_or("#<unknown>")
                    .to_owned(),
                function: atom_table
                    .resolve(function)
                    .unwrap_or("#<unknown>")
                    .to_owned(),
                arity,
                // A compiled frame has no interpreted instruction pointer — its
                // `ip` is a placeholder zero, and resolving a line from it would
                // report the module's FIRST line-table entry as though it were
                // the raise site. `build_stacktrace` has always guarded this;
                // this renderer did not, and a fabricated line is worse than an
                // absent one because it looks measured.
                line: (!entry.compiled)
                    .then(|| entry.module.line_at_ip(entry.ip))
                    .flatten(),
            }
        })
        .collect()
}

#[cfg(feature = "telemetry")]
const fn exit_reason_label(reason: ExitReason) -> &'static str {
    match reason {
        ExitReason::Normal => "normal",
        ExitReason::Kill => "kill",
        ExitReason::Killed => "killed",
        ExitReason::Error => "error",
        ExitReason::NoConnection => "noconnection",
        ExitReason::NoProc => "noproc",
        ExitReason::WorkerCrashed => "worker_crashed",
    }
}
pub(super) fn exit_reason_from_status(status: ProcessStatus) -> ExitReason {
    match status {
        ProcessStatus::Exited(reason) => reason,
        _ => ExitReason::Error,
    }
}

pub(in crate::scheduler) fn cleanup_exited_process(
    shared: &SharedState,
    pid: u64,
    reason: ExitReason,
) {
    shared.insert_exit_tombstone(pid, reason);
    let _deleted_tables = shared.transfer_or_delete_tables_owned_by(pid);
    supervision_integration::propagate_exit(shared, pid, reason);
    finalize_exited_process(shared, pid, reason);
}

/// Finalization half of process death: everything AFTER exit propagation.
///
/// Split from [`cleanup_exited_process`] so the link-cascade kill path
/// (`process_exit_signal`'s should-die arm) can finalize a target it has
/// already propagated for — calling the full cleanup there would
/// re-propagate links/monitors the cascade has consumed, while calling
/// nothing (the previous shape) left the process body, its owned fd
/// resources, its pg memberships, and its metric state stranded on every
/// cascade kill of a stored process.
///
/// Exactly-once, in two halves with two tokens:
///
/// - The **table token** (atomic process-table removal) guards the pid-keyed
///   work: lifecycle telemetry, metric state, wait-set sweep, timers,
///   suspension purge, pg purge. A concurrent loser returns immediately.
///   Reachable overlap: two `terminate_process` callers can both pass the
///   tombstone precheck and reach here, and a worker's tombstone cleanup can
///   overlap an external one.
/// - The **body token** (removing the `process_bodies` entry) guards the
///   body-owned work: fd close and refcounted-resource release. Resources are
///   released only on the `Present` body the removal itself yielded — never
///   in place — so a worker's `take_runnable_process` can never check out a
///   body whose resources this finalizer already released. When the removal
///   yields an `Executing` shadow, the worker owns the real body; its
///   store-back finds the entry gone, the table dead, and disposes of the
///   checked-out process itself ([`store_runnable_process`](super::store::store_runnable_process)). Table-token
///   consumption while the body is checked out is therefore safe: the body
///   half is finished by whoever actually holds the body.
pub(in crate::scheduler) fn finalize_exited_process(
    shared: &SharedState,
    pid: u64,
    reason: ExitReason,
) {
    if shared.process_table.remove(pid).is_none() {
        return;
    }
    shared.exit_completion_watches.fire(pid, reason);
    #[cfg(feature = "telemetry")]
    crate::telemetry::lifecycle::record_process_exited(&shared.atom_table, pid, reason);
    #[cfg(feature = "readiness")]
    shared.purge_readiness_state(pid);
    if let Some((_pid, slot_mutex)) = shared.process_bodies.remove(&pid) {
        let slot = slot_mutex
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let ProcessSlot::Present(ScheduledProcess(mut process)) = slot {
            release_process_exit_resources(&mut process, pid);
        }
    }
    #[cfg(feature = "telemetry")]
    {
        shared.remove_process_metric_state(pid);
        shared.record_scheduler_executing(std::time::Duration::ZERO);
    }
    let mut wait_set = lock_or_recover(&shared.wait_set);
    wait_set.waiting.remove(&pid);
    wait_set.woken.retain(|(woken_pid, _)| *woken_pid != pid);
    drop(wait_set);
    let _stale_marks = shared.expired_receive_timers.remove(&pid);
    // Purge every (pid, *) suspension structure — mirrors, published
    // completions, sticky resumes, file-I/O completions — so a dead pid can
    // neither leak entries nor have a late completion misattributed.
    shared.purge_suspension_state(pid);
    // Purge this pid from every pg group: the local removal runs synchronously
    // inside (an empty-group scan for non-pg processes is cheap and does no I/O),
    // and each resulting leave is propagated asynchronously through the installed
    // `SchedulerPgPropagation`/`DistSender`, so the exit path never blocks.
    //
    // CONTENTION NOTE: this acquires the GLOBAL `PgState` mutex on EVERY process
    // death — even for the common case of a process that joined no pg group,
    // where the scan finds nothing. Under high process churn concurrent with a
    // busy pg registry this is a single global-lock acquire on the hot exit path.
    // Accepted for now: the held critical section is a cheap membership scan with
    // no I/O, and exits are not the system's throughput-critical path. Revisit
    // (e.g. a per-pid "is in any group" fast-path flag to skip the lock) only if
    // profiling shows this lock as a real contention point.
    shared.pg_registry.remove_pid_from_all_scopes(pid);
}

/// Releases the fd and refcounted resources owned by an exiting process
/// body. Callers must hold the LAST reference to the body — either the
/// `Present` slot yielded by the finalizer's `process_bodies` removal, or
/// the checked-out process a worker's store-back found orphaned — so the
/// release can never run twice for one body.
pub(super) fn release_process_exit_resources(process: &mut Process, pid: u64) {
    process.heap().visit_boxed_objects(|ptr, tag, _words| {
        if tag == BoxedTag::FdResource {
            close_owned_resource_at(ptr, pid);
        }
    });
    release_all_refcounted_resources(process);
}

#[cfg(test)]
mod frame_resolution_tests {
    use super::resolve_raise_frames;
    use crate::atom::{Atom, AtomTable};
    use crate::loader::LineInfo;
    use crate::module::{Module, ModuleOrigin};
    use crate::process::{Process, RawStackEntry};
    use crate::term::Term;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn module_named(name: Atom) -> Module {
        Module {
            name,
            generation: 0,
            origin: ModuleOrigin::Preloaded,
            exports: HashMap::new(),
            label_index: HashMap::new(),
            code: Vec::new(),
            literals: Vec::new(),
            constant_pool: crate::constant_pool::ConstantPool::default(),
            resolved_imports: Vec::new(),
            lambdas: Vec::new(),
            string_table: Vec::new(),
            function_table: Vec::new(),
            line_table: Vec::new(),
            line_info: Vec::new(),
        }
    }

    /// A module whose first line-table entry is line 54 — the shape that makes
    /// a fabricated line look like a measured one.
    /// A module whose line table has the shape a real one has: the first `line`
    /// marker sits at ip 1, because ip 0 is the leading `label` of the first
    /// function's prologue (`label → line → func_info`).
    ///
    /// This previously put the marker at ip **0**, which no module produced by
    /// the loader is observed to do — measured across every `.beam` in
    /// `test-workflows/sample` (observed behaviour, not a loader-enforced
    /// rule), and gated by
    /// `loader::load::tests::no_real_module_carries_a_line_marker_at_ip_zero`. A fixture the
    /// real producer cannot build proves a defect in a world that does not
    /// exist, and that is exactly what it did here: it made `line_at_ip(0)`
    /// return a line, which on a real module it never can.
    fn module_with_realistic_line_table(name: Atom, first_line: u32) -> Module {
        let mut module = module_named(name);
        module.line_table = vec![(1, 0)];
        module.line_info = vec![LineInfo {
            file: 0,
            line: first_line,
        }];
        module
    }

    fn compiled_frame(pinned: &Arc<Module>, mfa: (Atom, Atom, u8)) -> RawStackEntry {
        RawStackEntry {
            module: Arc::clone(pinned),
            // `jit_add_compiled_frame` hardcodes this: a compiled frame has no
            // interpreted instruction pointer to record.
            ip: 0,
            mfa: Some(mfa),
            location_info: Term::NIL,
            compiled: true,
        }
    }

    #[test]
    fn compiled_frame_reports_the_module_owning_the_compiled_function() {
        // The process is positioned in the CALLER while compiled code from the
        // CALLEE's module raises. Taking the module from the pinned side spliced
        // the caller's module name onto the callee's function, producing a frame
        // naming a function that does not exist in the module it claims.
        let atom_table = AtomTable::new();
        let caller = atom_table.intern("aion@awl@codec");
        let callee = atom_table.intern("gleam@json");
        let function = atom_table.intern("int");

        let pinned = Arc::new(module_named(caller));
        let mut process = Process::new(1, 128);
        process.set_raw_stacktrace(vec![compiled_frame(&pinned, (callee, function, 1))]);

        let frames = resolve_raise_frames(&atom_table, &process);

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].module, "gleam@json");
        assert_eq!(frames[0].function, "int");
        assert_eq!(frames[0].arity, 1);
    }

    #[test]
    fn compiled_frame_reports_no_line_from_its_placeholder_ip() {
        // ⚠️ THIS ARM DOES NOT DISCRIMINATE, and says so rather than being
        // quietly counted. A compiled frame's ip is a placeholder zero, and on
        // any realistic line table `line_at_ip(0)` is already `None` — so this
        // assertion holds both with the `!compiled` guard and without it. The
        // guard is still right: it makes the absent line structural instead of
        // a coincidence of where the first marker happens to sit. But removing
        // the guard would NOT turn this red, and an earlier version of this
        // test only appeared to discriminate because its fixture put a line
        // marker at ip 0 — a shape the loader never produces.
        let atom_table = AtomTable::new();
        let owner = atom_table.intern("gleam_json_ffi");
        let function = atom_table.intern("int");

        let pinned = Arc::new(module_with_realistic_line_table(owner, 54));
        // The coincidence this guard removes reliance on: with the first marker
        // at ip 1, a lookup at the placeholder ip already finds nothing.
        assert_eq!(pinned.line_at_ip(0), None);

        let mut process = Process::new(1, 128);
        process.set_raw_stacktrace(vec![compiled_frame(&pinned, (owner, function, 1))]);

        let frames = resolve_raise_frames(&atom_table, &process);

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].module, "gleam_json_ffi");
        assert_eq!(frames[0].line, None);
    }

    #[test]
    fn interpreted_frames_keep_resolving_their_line_from_the_instruction_pointer() {
        // The compiled-frame guard must not blind the interpreted path: this is
        // the arm that would go quiet if the guard were written the wrong way
        // round.
        let atom_table = AtomTable::new();
        let owner = atom_table.intern("gleam_json_ffi");

        let mut module = module_with_realistic_line_table(owner, 54);
        module.function_table = vec![(0, atom_table.intern("int"), 1)];
        let pinned = Arc::new(module);

        let mut process = Process::new(1, 128);
        process.set_raw_stacktrace(vec![RawStackEntry {
            module: Arc::clone(&pinned),
            // ip 2 is a realistic interpreted position: inside the function,
            // past its prologue `line` marker at ip 1. An interpreted frame's
            // ip is a real position, never the compiled path's placeholder 0.
            ip: 2,
            mfa: None,
            location_info: Term::NIL,
            compiled: false,
        }]);

        let frames = resolve_raise_frames(&atom_table, &process);

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].module, "gleam_json_ffi");
        assert_eq!(frames[0].function, "int");
        assert_eq!(frames[0].line, Some(54));
    }
}
