//! The interpreter — the execution loop and heartbeat of fairness.
//!
//! Fetch, decode, execute, decrement reduction counter. When the
//! counter hits zero, save state and yield. Implements the subset
//! of BEAM opcodes that Gleam actually emits (per D5).
pub mod opcodes;
pub mod pattern;

use crate::error::ExecError;
use crate::module::{Module, ModuleRegistry};
use crate::process::{CodePosition, ExitReason, Process};
use crate::scheduler::dirty::DirtyCall;

/// Result of running a process until it yields, waits, exits, or faults.
#[derive(Debug)]
pub enum ExecutionResult {
    /// Reduction budget exhausted; scheduler should reset and requeue.
    Yielded,
    /// Process blocked waiting for a receive-family opcode.
    Waiting,
    /// Process entered the dirty scheduler for a native call.
    DirtyCall(DirtyCall),
    /// Process terminated with an exit reason.
    Exited(ExitReason),
}

impl PartialEq for ExecutionResult {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Yielded, Self::Yielded) | (Self::Waiting, Self::Waiting) => true,
            (Self::Exited(left), Self::Exited(right)) => left == right,
            (Self::DirtyCall(_), Self::DirtyCall(_)) => true,
            _ => false,
        }
    }
}

impl Eq for ExecutionResult {}

/// Control-flow outcome from one atomically completed instruction.
#[derive(Debug)]
pub enum InstructionOutcome {
    /// Continue at the sequential next instruction.
    Continue,
    /// Jump to a non-sequential code position.
    Jump(CodePosition),
    /// Yield after preserving the next code position in the process.
    Yield,
    /// Block waiting for a message.
    Waiting,
    /// Dispatch a dirty native call to the dirty scheduler.
    DirtyCall(DirtyCall),
    /// Exit the process.
    Exit(ExitReason),
}

impl PartialEq for InstructionOutcome {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Continue, Self::Continue)
            | (Self::Yield, Self::Yield)
            | (Self::Waiting, Self::Waiting) => true,
            (Self::Jump(left), Self::Jump(right)) => left == right,
            (Self::Exit(left), Self::Exit(right)) => left == right,
            (Self::DirtyCall(_), Self::DirtyCall(_)) => true,
            _ => false,
        }
    }
}

impl Eq for InstructionOutcome {}

/// Execute `process` against `module` until a scheduler boundary or exit.
pub fn run(process: &mut Process, module: &Module) -> Result<ExecutionResult, ExecError> {
    run_loop(process, module, None)
}

/// Execute `process` with a registry so dynamic calls can cross module boundaries.
pub fn run_with_registry(
    process: &mut Process,
    initial_module: &Module,
    registry: &ModuleRegistry,
) -> Result<ExecutionResult, ExecError> {
    run_loop(process, initial_module, Some(registry))
}

fn run_loop(
    process: &mut Process,
    initial_module: &Module,
    registry: Option<&ModuleRegistry>,
) -> Result<ExecutionResult, ExecError> {
    if process.code_position().is_none() {
        process.set_code_position(Some(CodePosition {
            module: initial_module.name,
            instruction_pointer: 0,
        }));
    }

    loop {
        let position = process
            .code_position()
            .ok_or(ExecError::InvalidOperand("code position"))?;
        let module_guard = registry.and_then(|registry| registry.lookup(position.module));
        let module = module_guard.as_deref().unwrap_or(initial_module);
        if module.name != position.module {
            return Err(ExecError::InvalidOperand("code position module"));
        }
        let instruction = module
            .code
            .get(position.instruction_pointer)
            .ok_or(ExecError::InvalidOperand("instruction pointer"))?;
        let next_ip = position
            .instruction_pointer
            .checked_add(1)
            .ok_or(ExecError::InvalidOperand("instruction pointer"))?;

        match opcodes::dispatch(process, module, instruction, next_ip)? {
            InstructionOutcome::Continue => process.set_code_position(Some(CodePosition {
                module: module.name,
                instruction_pointer: next_ip,
            })),
            InstructionOutcome::Jump(target) => process.set_code_position(Some(target)),
            InstructionOutcome::Yield => return Ok(ExecutionResult::Yielded),
            InstructionOutcome::Waiting => return Ok(ExecutionResult::Waiting),
            InstructionOutcome::DirtyCall(call) => {
                process.set_code_position(Some(CodePosition {
                    module: module.name,
                    instruction_pointer: next_ip,
                }));
                return Ok(ExecutionResult::DirtyCall(call));
            }
            InstructionOutcome::Exit(reason) => {
                process.set_code_position(None);
                return Ok(ExecutionResult::Exited(reason));
            }
        }
    }
}

#[cfg(test)]
mod tests;
