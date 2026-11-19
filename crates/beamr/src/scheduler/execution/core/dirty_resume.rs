//! Resuming a process from a dirty native call: parking it while the call is
//! in flight, and applying the call's result or trampoline when it lands.

use crate::atom::Atom;
use crate::native::ExceptionClass;
use crate::process::{CodePosition, ExitReason, Process, SuspensionKind, SuspensionRecord};
use crate::scheduler::dirty::DirtyResult;
use crate::scheduler::{SharedState, namespace_registry};
use crate::term::Term;
use std::sync::Arc;

use super::suspension_gate::{SuspensionGate, advance_past_current_instruction};

/// Re-park a process whose dirty native requested suspension: install a NEW
/// host-await suspension at the (unadvanced) dirty call instruction. The
/// completion arrives through the pid-resolved `Scheduler::wake_with_result`
/// (or `wake_with_result_for` once the embedder learns the id), the optional
/// timeout re-executes the dirty call, and — for a message-wakeable request —
/// any message re-executes the (re-entrant) dirty call.
pub(super) fn apply_dirty_suspend(
    shared: &SharedState,
    process: &mut Process,
    suspend: crate::native::SuspendRequest,
) -> SuspensionGate {
    let Some(position) = process.code_position() else {
        return SuspensionGate::Error(crate::error::ExecError::InvalidOperand(
            "dirty suspend code position",
        ));
    };
    if let Some(timeout_ms) = suspend.timeout_ms {
        process.set_receive_timeout(Some(crate::process::ReceiveTimeout {
            timeout_position: position,
            milliseconds: timeout_ms,
        }));
    }
    // A detached dirty context cannot allocate from the process counter;
    // the id is allocated here, on the owning thread.
    let call_id = suspend
        .call_id
        .unwrap_or_else(|| process.allocate_suspension_call_id());
    // Dirty tail calls (`call_ext_last`/`call_ext_only`) had their y-frame
    // popped eagerly before submission, so a host result applied at this
    // park returns to the caller without another pop.
    let continuation = match parked_instruction(shared, process, position) {
        Some(
            crate::loader::Instruction::CallExtLast { .. }
            | crate::loader::Instruction::CallExtOnly { .. },
        ) => crate::process::ResumeContinuation::Return,
        _ => crate::process::ResumeContinuation::Advance,
    };
    process.set_suspension(Some(SuspensionRecord {
        call_id,
        kind: SuspensionKind::HostAwait,
        position: Some(position),
        wake_on_message: suspend.wake_on_message,
        continuation,
    }));
    shared.register_suspension_mirror(
        process.pid(),
        call_id,
        SuspensionKind::HostAwait,
        suspend.wake_on_message,
    );
    SuspensionGate::Repark(SuspensionKind::HostAwait)
}

/// The instruction a suspension parked at, cloned out of the owning module.
fn parked_instruction(
    shared: &SharedState,
    process: &Process,
    position: CodePosition,
) -> Option<crate::loader::Instruction> {
    let module = if let Some(current) = process
        .current_module()
        .filter(|module| module.name == position.module)
    {
        Arc::clone(current)
    } else {
        let registry = namespace_registry(shared, process.namespace_id())
            .unwrap_or_else(|| Arc::clone(&shared.module_registry));
        registry.lookup(position.module)?
    };
    module.code.get(position.instruction_pointer).cloned()
}

/// Set up the closure call a dirty native requested: copy the owned fun and
/// arguments onto the resuming process heap and run the normal trampoline
/// path (which pushes the continuation under the suspension protocol's
/// position-gated readiness check).
pub(super) fn apply_dirty_trampoline(
    shared: &SharedState,
    process: &mut Process,
    trampoline: crate::scheduler::dirty::OwnedDirtyTrampoline,
) -> SuspensionGate {
    let registry = namespace_registry(shared, process.namespace_id())
        .unwrap_or_else(|| Arc::clone(&shared.module_registry));
    let Some(position) = process.code_position() else {
        return SuspensionGate::Error(crate::error::ExecError::InvalidOperand(
            "dirty trampoline code position",
        ));
    };
    let module = if let Some(current) = process
        .current_module()
        .filter(|m| m.name == position.module)
    {
        Arc::clone(current)
    } else if let Some(module) = registry.lookup(position.module) {
        module
    } else {
        return SuspensionGate::Error(crate::error::ExecError::InvalidOperand(
            "dirty trampoline module",
        ));
    };
    let badarg = |_| crate::error::ExecError::Badarg;
    let fun = match trampoline
        .fun
        .copy_to_heap(process.heap_mut())
        .map_err(badarg)
    {
        Ok(fun) => fun,
        Err(error) => return SuspensionGate::Error(error),
    };
    let mut args = Vec::with_capacity(trampoline.args.len());
    for arg in &trampoline.args {
        match arg.copy_to_heap(process.heap_mut()).map_err(badarg) {
            Ok(arg) => args.push(arg),
            Err(error) => return SuspensionGate::Error(error),
        }
    }
    match crate::interpreter::opcodes::trampoline::handle_trampoline(
        process,
        &module,
        Some(&registry),
        crate::native::TrampolineRequest {
            fun,
            args,
            continuation: Some(trampoline.continuation),
        },
    ) {
        // Jump: the closure entry becomes the slice's starting position.
        Ok(crate::interpreter::InstructionOutcome::Jump(target)) => {
            process.set_code_position(Some(target));
            SuspensionGate::Run
        }
        // Yield: handle_trampoline already stored the target position.
        Ok(_) => SuspensionGate::Run,
        Err(error) => SuspensionGate::Error(error),
    }
}

pub(super) enum InstructionOutcomeAfterDirty {
    Continue,
    Exit(ExitReason),
}

pub(super) fn apply_dirty_result(
    process: &mut Process,
    dirty_result: DirtyResult,
) -> Result<InstructionOutcomeAfterDirty, crate::error::ExecError> {
    let owned_result = dirty_result.owned_result;
    // Collect/grow before materializing the owned result on the process
    // heap — `copy_to_heap` itself cannot trigger GC, so an arbitrarily
    // large dirty result (e.g. a big binary) must not die on HeapFull.
    if let Some(owned) = owned_result.as_ref() {
        crate::gc::ensure_space(process, owned.total_words(), 256)
            .map_err(|_| crate::error::ExecError::Badarg)?;
    }
    match dirty_result.result {
        Ok(value) => {
            let value = match owned_result.as_ref() {
                Some(owned) => owned
                    .copy_to_heap(process.heap_mut())
                    .map_err(|_| crate::error::ExecError::Badarg)?,
                None => value,
            };
            process.set_x_reg(0, value);
            advance_past_current_instruction(process);
            Ok(InstructionOutcomeAfterDirty::Continue)
        }
        Err(reason) => {
            let reason = match owned_result.as_ref() {
                Some(owned) => owned
                    .copy_to_heap(process.heap_mut())
                    .map_err(|_| crate::error::ExecError::Badarg)?,
                None => reason,
            };
            let exception = crate::process::Exception {
                class: Term::atom(exception_class_atom(dirty_result.exception_class)),
                reason,
                stacktrace: dirty_result.exception_stacktrace,
            };
            match crate::interpreter::opcodes::exceptions::raise_exception(process, exception)? {
                crate::interpreter::InstructionOutcome::Jump(target) => {
                    process.set_code_position(Some(target));
                    Ok(InstructionOutcomeAfterDirty::Continue)
                }
                crate::interpreter::InstructionOutcome::Exit(reason) => {
                    Ok(InstructionOutcomeAfterDirty::Exit(reason))
                }
                crate::interpreter::InstructionOutcome::Continue
                | crate::interpreter::InstructionOutcome::Yield
                | crate::interpreter::InstructionOutcome::Waiting
                | crate::interpreter::InstructionOutcome::NativeContinuation
                | crate::interpreter::InstructionOutcome::OnLoadComplete
                | crate::interpreter::InstructionOutcome::DirtyCall { .. } => {
                    Ok(InstructionOutcomeAfterDirty::Exit(ExitReason::Error))
                }
            }
        }
    }
}
fn exception_class_atom(class: ExceptionClass) -> Atom {
    match class {
        ExceptionClass::Error => Atom::ERROR,
        ExceptionClass::Throw => Atom::THROW,
        ExceptionClass::Exit => Atom::EXIT_CLASS,
    }
}
