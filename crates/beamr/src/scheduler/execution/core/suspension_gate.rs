//! The slice-start suspension gate: whether a parked process may run now,
//! and how a completion applied at its park position continues execution.

use crate::process::{CodePosition, ExitReason, Process, ProcessStatus, SuspensionKind};
use crate::scheduler::suspension::{self, SuspensionResultPayload};
use crate::scheduler::{SharedState, timer_integration};
use std::sync::Arc;

use super::SliceOutcome;
use super::dirty_resume::{
    InstructionOutcomeAfterDirty, apply_dirty_result, apply_dirty_suspend, apply_dirty_trampoline,
};
use super::slice::take_process;

/// Decision produced by the slice-start suspension gate.
pub(super) enum SuspensionGate {
    /// No live suspension, or its event was consumed: run the interpreter.
    Run,
    /// The suspension has no consumable event: park again untouched.
    Repark(SuspensionKind),
    /// Applying a dirty exception unwound the process to an exit.
    Exit(ExitReason),
    /// Applying the completion failed.
    Error(crate::error::ExecError),
}

/// Consume `process`'s pending suspension event, if any. See the call site
/// in [`execute_slice_with_budget`] for the protocol contract.
pub(super) fn consume_suspension_event(
    shared: &SharedState,
    process: &mut Process,
) -> SuspensionGate {
    let pid = process.pid();
    let Some(record) = process.suspension() else {
        // No live suspension: any published completion or mirror is the
        // orphan of an abandoned/superseded suspend request — drop it,
        // then apply a plain receive timer fire.
        let _stale_mirror = shared.suspensions.remove(&pid);
        let _orphan_result = shared.suspension_results.remove(&pid);
        let _jumped = timer_integration::apply_expired_receive_timer(shared, process);
        return SuspensionGate::Run;
    };
    if let Some((_, result)) = shared
        .suspension_results
        .remove_if(&pid, |_, result| result.call_id == record.call_id)
    {
        let _consumed = process.take_suspension();
        shared.suspensions.remove(&pid);
        if record.position != process.code_position() {
            // The identity matched but the park position moved: a protocol
            // violation. Refuse to mutate the instruction pointer blind —
            // that desync is exactly the "invalid operand for instruction
            // pointer" crash class this gate exists to prevent.
            debug_assert_eq!(
                record.position,
                process.code_position(),
                "suspension result applied at a different position than it was produced"
            );
            return SuspensionGate::Error(crate::error::ExecError::InvalidOperand(
                "suspension result position",
            ));
        }
        // The completion owns the timed-await lifecycle: clear the timeout
        // metadata so a raced timer fire is dropped as stale and a later
        // plain wait cannot arm a timer at this stale resume position.
        process.set_receive_timeout(None);
        process.set_receive_timer_ref(None);
        let _stale_marks = shared.expired_receive_timers.remove(&pid);
        return match result.payload {
            SuspensionResultPayload::Host(owned) => {
                // The published result owns its boxed storage; materialize
                // it on the resuming process heap so x0 never points into
                // memory with a foreign lifetime. Collect/grow first —
                // `copy_to_heap` itself cannot trigger GC.
                if crate::gc::ensure_space(process, owned.total_words(), 256).is_err() {
                    return SuspensionGate::Error(crate::error::ExecError::InvalidOperand(
                        "suspension result heap space",
                    ));
                }
                let term = match owned.copy_to_heap(process.heap_mut()) {
                    Ok(term) => term,
                    Err(_error) => {
                        return SuspensionGate::Error(crate::error::ExecError::InvalidOperand(
                            "suspension result term",
                        ));
                    }
                };
                process.set_x_reg(0, term);
                continue_past_applied_result(process, record.continuation)
            }
            SuspensionResultPayload::Dirty(mut dirty_result) => {
                // Follow-up requests from the dirty native (B-5b): a
                // re-suspend re-parks at the dirty call instruction under a
                // NEW host-await suspension; a trampoline sets up the
                // requested closure call. Only honored on the Ok path (the
                // exception wins, matching call_native_entry).
                if dirty_result.result.is_ok() {
                    if let Some(suspend) = dirty_result.suspend.take() {
                        return apply_dirty_suspend(shared, process, suspend);
                    }
                    if let Some(trampoline) = dirty_result.trampoline.take() {
                        return apply_dirty_trampoline(shared, process, trampoline);
                    }
                }
                match apply_dirty_result(process, *dirty_result) {
                    Ok(InstructionOutcomeAfterDirty::Continue) => SuspensionGate::Run,
                    Ok(InstructionOutcomeAfterDirty::Exit(reason)) => SuspensionGate::Exit(reason),
                    Err(error) => SuspensionGate::Error(error),
                }
            }
        };
    }
    // A published completion that did not match above is stale — produced
    // for an earlier suspension whose await already timed out — and must
    // never be applied. (remove_if keeps a *matching* completion that lands
    // concurrently: its wake re-schedules this process.)
    let _stale_result = shared
        .suspension_results
        .remove_if(&pid, |_, result| result.call_id != record.call_id);
    match record.kind {
        SuspensionKind::HostAwait => {
            if shared.file_io_results.contains_key(&pid) {
                // A ring completion resumes the await by re-executing the
                // native at the park position; the native consumes it via
                // take_file_io_completion. The suspension itself is done.
                let _consumed = process.take_suspension();
                shared.suspensions.remove(&pid);
                process.set_receive_timeout(None);
                process.set_receive_timer_ref(None);
                let _stale_marks = shared.expired_receive_timers.remove(&pid);
                SuspensionGate::Run
            } else if timer_integration::apply_expired_receive_timer(shared, process) {
                // Timed out: the native re-executes at the recorded timeout
                // position. receive_timeout stays set so the native observes
                // receive_timeout_expired and reports the timeout. This
                // suspension is superseded; its late completion becomes an
                // orphan and is dropped, never applied.
                let _consumed = process.take_suspension();
                shared.suspensions.remove(&pid);
                SuspensionGate::Run
            } else if record.wake_on_message {
                // Message-wakeable suspend (select, marker awaits): any
                // wake re-executes the re-entrant native, which scans the
                // mailbox and may suspend again under a NEW call id. The
                // current suspension ends here; a completion later
                // published for it is stale and will be dropped.
                let _consumed = process.take_suspension();
                shared.suspensions.remove(&pid);
                SuspensionGate::Run
            } else {
                SuspensionGate::Repark(SuspensionKind::HostAwait)
            }
        }
        SuspensionKind::DirtyCall => SuspensionGate::Repark(SuspensionKind::DirtyCall),
        SuspensionKind::Hook => {
            let matching_resume = shared.pending_resumes.remove_if(&pid, |_, resume| {
                *resume == suspension::RESUME_ANY_HOOK || *resume == record.call_id
            });
            if matching_resume.is_some() {
                let _consumed = process.take_suspension();
                shared.suspensions.remove(&pid);
                let _jumped = timer_integration::apply_expired_receive_timer(shared, process);
                SuspensionGate::Run
            } else {
                // A resume targeting an older hook suspension is stale.
                let _stale_resume = shared.pending_resumes.remove_if(&pid, |_, resume| {
                    *resume != suspension::RESUME_ANY_HOOK && *resume != record.call_id
                });
                SuspensionGate::Repark(SuspensionKind::Hook)
            }
        }
    }
}

/// Park a gated process again without running it: back to Waiting (host
/// awaits park through the Wait arm) or Suspended (dirty calls and hook
/// suspends park through the Suspended arm).
pub(super) fn repark_suspended(process: &mut Process, kind: SuspensionKind) -> SliceOutcome {
    match kind {
        SuspensionKind::HostAwait => {
            let _t = process.transition_to(ProcessStatus::Waiting);
            SliceOutcome::Wait(take_process(process))
        }
        SuspensionKind::DirtyCall | SuspensionKind::Hook => {
            let _t = process.transition_to(ProcessStatus::Suspended);
            SliceOutcome::Suspended(take_process(process))
        }
    }
}

pub(super) fn advance_past_current_instruction(process: &mut Process) {
    if let Some(pos) = process.code_position() {
        process.set_code_position(Some(CodePosition {
            module: pos.module,
            instruction_pointer: pos.instruction_pointer.saturating_add(1),
        }));
    }
}

/// Continue execution after a published host result was applied into x0 at
/// a parked call instruction.
///
/// Body-position parks resume at the next instruction. Tail-position parks
/// (`call_ext_only` / `call_ext_last`) treat the applied result as the
/// function's return value: the process returns to the caller — popping
/// the deferred `call_ext_last` y-frame first — instead of running off the
/// end of the function. An empty stack means the applied result IS the
/// process's final value.
fn continue_past_applied_result(
    process: &mut Process,
    continuation: crate::process::ResumeContinuation,
) -> SuspensionGate {
    match continuation {
        crate::process::ResumeContinuation::Advance => {
            advance_past_current_instruction(process);
            SuspensionGate::Run
        }
        crate::process::ResumeContinuation::Return => return_to_caller(process),
        crate::process::ResumeContinuation::DeallocateAndReturn => {
            if process.stack_mut().pop_frame().is_err() {
                return SuspensionGate::Error(crate::error::ExecError::InvalidOperand(
                    "suspension result deferred frame",
                ));
            }
            return_to_caller(process)
        }
    }
}

/// Pop the caller's return frame and jump there (the slice-start mirror of
/// the interpreter's `return` instruction).
fn return_to_caller(process: &mut Process) -> SuspensionGate {
    if process.stack().is_empty() {
        return SuspensionGate::Exit(ExitReason::Normal);
    }
    let return_point = match process.stack_mut().pop_frame() {
        Ok(return_point) => return_point,
        Err(_error) => {
            return SuspensionGate::Error(crate::error::ExecError::InvalidOperand(
                "suspension result return frame",
            ));
        }
    };
    process.set_current_module(Arc::clone(&return_point.module_version));
    process.set_code_position(Some(CodePosition {
        module: return_point.module,
        instruction_pointer: return_point.ip,
    }));
    SuspensionGate::Run
}
