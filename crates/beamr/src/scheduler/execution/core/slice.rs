//! One time slice of a process: entering it, running the interpreter for the
//! budget, and mapping how the slice ended to a `SliceOutcome`.

use crate::hook::HookDecision;
use crate::interpreter::{self, ExecutionResult};
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::process::{ExitReason, Process, ProcessStatus, SuspensionKind, SuspensionRecord};
use crate::replay::RecordedSchedule;
use crate::scheduler::dirty::DirtySchedulerKind;
use crate::scheduler::{
    DEFAULT_REDUCTION_BUDGET, SharedState, namespace_registry, supervision_integration,
    timer_integration,
};
use std::sync::Arc;

use super::SliceOutcome;
use super::dirty_submit::{DirtyInvocation, DirtySubmissionError, submit_dirty_call};
use super::exit::{exit_process, exit_reason_from_status};
use super::replay_schedule::validate_replay_schedule_reductions;
use super::suspension_gate::{SuspensionGate, consume_suspension_event, repark_suspended};

pub(in crate::scheduler) fn execute_slice(
    shared: &Arc<SharedState>,
    process: &mut Process,
) -> SliceOutcome {
    execute_slice_with_budget(shared, process, None)
}

pub(in crate::scheduler) fn execute_slice_with_recorded_schedule(
    shared: &Arc<SharedState>,
    process: &mut Process,
    recorded_schedule: RecordedSchedule,
) -> SliceOutcome {
    execute_slice_with_budget(shared, process, Some(recorded_schedule))
}

fn execute_slice_with_budget(
    shared: &Arc<SharedState>,
    process: &mut Process,
    recorded_schedule: Option<RecordedSchedule>,
) -> SliceOutcome {
    if !matches!(
        process.status(),
        ProcessStatus::New
            | ProcessStatus::Yielded
            | ProcessStatus::Waiting
            | ProcessStatus::Suspended
    ) {
        return SliceOutcome::Exited(
            exit_reason_from_status(process.status()),
            crate::scheduler::exit_capture::capture_term(process.x_reg(0)),
        );
    }
    if transition_to_running(process).is_err() {
        return SliceOutcome::Exited(
            exit_reason_from_status(process.status()),
            crate::scheduler::exit_capture::capture_term(process.x_reg(0)),
        );
    }
    #[cfg(feature = "telemetry")]
    let span = start_slice_span(shared, process);
    // Suspension gate: a result-gated suspension (host await, dirty call,
    // hook suspend) is consumed here, on the owning thread, only by its
    // matching event — the completion published under the suspension's call
    // id, a file-I/O completion, the receive timeout, or a matching embedder
    // resume. Stale completions are dropped; with nothing consumable the
    // process re-parks untouched, so a stray wake can never re-execute the
    // parked call instruction (and double-submit its side effect).
    match consume_suspension_event(shared, process) {
        SuspensionGate::Run => {}
        SuspensionGate::Repark(kind) => {
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                0,
                crate::telemetry::spans::SliceSpanOutcome::Waiting,
            );
            return repark_suspended(process, kind);
        }
        SuspensionGate::Exit(reason) => {
            let outcome = exit_process(shared, process, reason);
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                0,
                crate::telemetry::spans::SliceSpanOutcome::Exited,
            );
            return outcome;
        }
        SuspensionGate::Error(error) => {
            let pid = process.pid();
            shared.exit_errors.insert(pid, error);
            let outcome = exit_process(shared, process, ExitReason::Error);
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                0,
                crate::telemetry::spans::SliceSpanOutcome::Exited,
            );
            return outcome;
        }
    }
    let reduction_budget = recorded_schedule.map_or(DEFAULT_REDUCTION_BUDGET, |recorded| {
        recorded.reduction_budget
    });
    process.reset_reductions(reduction_budget);
    let module_atom = match process.code_position() {
        Some(position) => position.module,
        None => {
            let outcome = exit_process(shared, process, ExitReason::Normal);
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                0,
                crate::telemetry::spans::SliceSpanOutcome::Exited,
            );
            return outcome;
        }
    };
    let registry = namespace_registry(shared, process.namespace_id())
        .unwrap_or_else(|| Arc::clone(&shared.module_registry));
    let module = if let Some(current) = process.current_module()
        && current.name == module_atom
        && current
            .code
            .get(
                process
                    .code_position()
                    .map_or(usize::MAX, |pos| pos.instruction_pointer),
            )
            .is_some()
    {
        Arc::clone(current)
    } else {
        let Some(module) = registry.lookup(module_atom) else {
            let outcome = exit_process(shared, process, ExitReason::Error);
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                0,
                crate::telemetry::spans::SliceSpanOutcome::Exited,
            );
            return outcome;
        };
        process.set_current_module(Arc::clone(&module));
        module
    };
    let services = supervision_integration::build_native_services(shared, process.namespace_id());
    let result = interpreter::run_with_native_services(process, &module, &registry, &services);
    let reductions = reduction_budget.saturating_sub(process.reduction_counter());
    if validate_replay_schedule_reductions(shared, recorded_schedule, reductions).is_err() {
        let outcome = exit_process(shared, process, ExitReason::Error);
        #[cfg(feature = "telemetry")]
        finish_slice_span(
            shared,
            process,
            span,
            reductions,
            crate::telemetry::spans::SliceSpanOutcome::Exited,
        );
        return outcome;
    }
    if matches!(
        result,
        Ok(ExecutionResult::Yielded) | Ok(ExecutionResult::Waiting)
    ) && timer_integration::invoke_hook(shared, process, reductions) == HookDecision::Suspend
        // A hook suspend must NOT target an await-parked slice: a Waiting
        // slice that parked through request_suspend/request_await_suspend
        // already carries a result-gated suspension record, and installing
        // the Hook record over it would invalidate the await's call id —
        // published completions would be dropped as stale and the eventual
        // embedder resume would re-execute the parked await native,
        // double-submitting its host side effect. The hook still observes
        // the slice; its Suspend decision is ignored and the process parks
        // under the await it requested.
        && process.suspension().is_none()
    {
        // Record the hook suspension's identity before parking so an
        // embedder resume_process targets exactly this suspension and a
        // resume racing the park gap is consumed by the slice-start gate.
        let call_id = process.allocate_suspension_call_id();
        process.set_suspension(Some(SuspensionRecord {
            call_id,
            kind: SuspensionKind::Hook,
            position: process.code_position(),
            wake_on_message: false,
            continuation: crate::process::ResumeContinuation::Advance,
        }));
        shared.register_suspension_mirror(process.pid(), call_id, SuspensionKind::Hook, false);
        let _t = process.transition_to(ProcessStatus::Suspended);
        #[cfg(feature = "telemetry")]
        finish_slice_span(
            shared,
            process,
            span,
            reductions,
            crate::telemetry::spans::SliceSpanOutcome::Waiting,
        );
        return SliceOutcome::Suspended(take_process(process));
    }
    match result {
        Ok(ExecutionResult::Yielded) => {
            let _t = process.transition_to(ProcessStatus::Yielded);
            process.reset_reductions(reduction_budget);
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                reductions,
                crate::telemetry::spans::SliceSpanOutcome::Yielded,
            );
            SliceOutcome::Requeue(take_process(process))
        }
        Ok(ExecutionResult::Waiting) => {
            let _t = process.transition_to(ProcessStatus::Waiting);
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                reductions,
                crate::telemetry::spans::SliceSpanOutcome::Waiting,
            );
            SliceOutcome::Wait(take_process(process))
        }
        Ok(ExecutionResult::DirtyCall {
            entry,
            args,
            module,
            function,
            arity,
            kind,
        }) => {
            // Refusal precedes suspension registration AND pool submit AND the
            // completion-bridge spawn (spec §3.2, normative ordering). A
            // Disabled pool has no worker to run the job, so a gated suspension
            // registered against it would never be completed — and per the
            // readiness contract's C2 scope limit no message can wake it, so
            // the process would park FOREVER. Refuse here, before any of those
            // side effects: the process terminates promptly with a typed
            // service-unavailable error instead.
            let target_available = match kind {
                DirtySchedulerKind::Cpu => shared.dirty_cpu.service().is_some(),
                DirtySchedulerKind::Io => shared.dirty_io.service().is_some(),
            };
            // Admission reservation, LINEARIZED with intake closure (spec §4
            // step 1/3): the reservation and the shutdown drain's close share
            // one lock, so a submission cannot be admitted behind the drain.
            // Reserved (or refused) HERE, before the call-id allocation below,
            // so the §3.2 refusal-before-suspension ordering (and its counter
            // gates) hold for this arm too. The reservation is an RAII token:
            // released on every error path below, converted into a retained,
            // OS-joinable bridge handle at spawn.
            let reservation = if target_available {
                shared.try_reserve_teardown_admission()
            } else {
                None
            };
            let Some(reservation) = reservation else {
                shared
                    .exit_errors
                    .insert(process.pid(), DirtySubmissionError::Disabled(kind).into());
                let outcome = exit_process(shared, process, ExitReason::Error);
                #[cfg(feature = "telemetry")]
                finish_slice_span(
                    shared,
                    process,
                    span,
                    reductions,
                    crate::telemetry::spans::SliceSpanOutcome::Exited,
                );
                return outcome;
            };
            // Record the dirty call's identity before submission so the
            // completion bridge publishes its result under this exact call
            // id and the wake gate holds the process parked meanwhile.
            //
            // The counter below instruments the FIRST side effect of the
            // gated-suspension sequence (call-id allocation), incremented
            // immediately before it so the §3.2 ordering gates hold it flat
            // across a refused call. Increment-first is the fail-closed
            // order: an availability check that regresses to between the
            // two lines moves the counter on a refused call and fails the
            // gate, rather than passing silently.
            #[cfg(any(test, feature = "test-support"))]
            shared
                .dirty_suspension_allocations
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let call_id = process.allocate_suspension_call_id();
            process.set_suspension(Some(SuspensionRecord {
                call_id,
                kind: SuspensionKind::DirtyCall,
                position: process.code_position(),
                wake_on_message: false,
                continuation: crate::process::ResumeContinuation::Advance,
            }));
            shared.register_suspension_mirror(
                process.pid(),
                call_id,
                SuspensionKind::DirtyCall,
                false,
            );
            if let Err(error) = submit_dirty_call(
                shared,
                process,
                DirtyInvocation {
                    call_id,
                    entry,
                    args,
                    mfa: (module, function, arity),
                    kind,
                },
                reservation,
            ) {
                let _withdrawn = process.take_suspension();
                shared.suspensions.remove(&process.pid());
                shared.exit_errors.insert(process.pid(), error.into());
                let outcome = exit_process(shared, process, ExitReason::Error);
                #[cfg(feature = "telemetry")]
                finish_slice_span(
                    shared,
                    process,
                    span,
                    reductions,
                    crate::telemetry::spans::SliceSpanOutcome::Exited,
                );
                return outcome;
            }
            let _t = process.transition_to(ProcessStatus::Suspended);
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                reductions,
                crate::telemetry::spans::SliceSpanOutcome::Waiting,
            );
            SliceOutcome::Suspended(take_process(process))
        }
        Ok(ExecutionResult::Exited(reason)) => {
            let outcome = exit_process(shared, process, reason);
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                reductions,
                crate::telemetry::spans::SliceSpanOutcome::Exited,
            );
            outcome
        }
        Err(error) => {
            let pid = process.pid();
            shared.exit_errors.insert(pid, error);
            let outcome = exit_process(shared, process, ExitReason::Error);
            #[cfg(feature = "telemetry")]
            finish_slice_span(
                shared,
                process,
                span,
                reductions,
                crate::telemetry::spans::SliceSpanOutcome::Exited,
            );
            outcome
        }
    }
}

/// Move a schedulable process to Running. A Suspended process reaches the
/// owning thread without the resume flip only on a spurious wake; route it
/// through Yielded so the lifecycle graph holds (the slice-start gate then
/// re-parks it unless a consumable event is pending).
fn transition_to_running(process: &mut Process) -> Result<(), ()> {
    if process.status() == ProcessStatus::Suspended
        && process.transition_to(ProcessStatus::Yielded).is_err()
    {
        return Err(());
    }
    process
        .transition_to(ProcessStatus::Running)
        .map_err(|_| ())
}

#[cfg(feature = "telemetry")]
fn start_slice_span(
    shared: &SharedState,
    process: &Process,
) -> crate::telemetry::spans::ExecutionSliceSpan {
    crate::telemetry::spans::ExecutionSliceSpan::start(&shared.atom_table, process)
}

#[cfg(feature = "telemetry")]
fn finish_slice_span(
    shared: &SharedState,
    process: &Process,
    span: crate::telemetry::spans::ExecutionSliceSpan,
    reductions_consumed: u32,
    outcome: crate::telemetry::spans::SliceSpanOutcome,
) {
    shared.record_process_slice_metrics(process, reductions_consumed);
    span.finish(&shared.atom_table, process, reductions_consumed, outcome);
}

pub(super) fn take_process(process: &mut Process) -> Process {
    std::mem::replace(process, Process::new(u64::MAX, DEFAULT_HEAP_SIZE))
}
