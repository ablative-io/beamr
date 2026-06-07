//! Floating-point opcode handlers.

use crate::error::ExecError;
use crate::gc::{self, GcError};
use crate::interpreter::InstructionOutcome;
use crate::loader::decode::Operand;
use crate::module::Module;
use crate::process::{Process, ProcessError};
use crate::term::Term;
use crate::term::boxed::{Float, write_float};

use super::core;

const FLOAT_BOX_WORDS: usize = 2;

pub fn fmove(
    process: &mut Process,
    module: &Module,
    source: &Operand,
    dest: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    match (source, dest) {
        (Operand::FloatRegister(_), Operand::FloatRegister(_)) => {
            let value = read_float_reg(process, source)?;
            write_float_reg(process, dest, value)?;
        }
        (Operand::FloatRegister(_), _) => {
            validate_term_destination(dest)?;
            let value = read_float_reg(process, source)?;
            let term = boxed_float(process, value)?;
            core::write_term(process, dest, term)?;
        }
        (_, Operand::FloatRegister(_)) => {
            let term = core::read_term(process, module, source)?;
            let value = Float::new(term)
                .map(Float::value)
                .ok_or(ExecError::Badarith)?;
            write_float_reg(process, dest, value)?;
        }
        _ => return Err(ExecError::InvalidOperand("fmove")),
    }

    Ok(InstructionOutcome::Continue)
}

pub fn fconv(
    process: &mut Process,
    module: &Module,
    source: &Operand,
    dest: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    let value = match source {
        Operand::FloatRegister(_) => read_float_reg(process, source)?,
        _ => number_to_float(core::read_term(process, module, source)?)?,
    };
    write_float_reg(process, dest, value)?;
    Ok(InstructionOutcome::Continue)
}

pub fn fadd(
    process: &mut Process,
    fail: &Operand,
    left: &Operand,
    right: &Operand,
    dest: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    let _ = fail;
    float_binop(process, left, right, dest, |left, right| left + right)
}

pub fn fsub(
    process: &mut Process,
    fail: &Operand,
    left: &Operand,
    right: &Operand,
    dest: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    let _ = fail;
    float_binop(process, left, right, dest, |left, right| left - right)
}

pub fn fmul(
    process: &mut Process,
    fail: &Operand,
    left: &Operand,
    right: &Operand,
    dest: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    let _ = fail;
    float_binop(process, left, right, dest, |left, right| left * right)
}

pub fn fdiv(
    process: &mut Process,
    fail: &Operand,
    left: &Operand,
    right: &Operand,
    dest: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    let _ = fail;
    let denominator = read_float_reg(process, right)?;
    if denominator == 0.0 {
        return Err(ExecError::Badarith);
    }
    let numerator = read_float_reg(process, left)?;
    write_float_reg(process, dest, numerator / denominator)?;
    Ok(InstructionOutcome::Continue)
}

pub fn fnegate(
    process: &mut Process,
    fail: &Operand,
    source: &Operand,
    dest: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    let _ = fail;
    let value = read_float_reg(process, source)?;
    write_float_reg(process, dest, -value)?;
    Ok(InstructionOutcome::Continue)
}

fn float_binop(
    process: &mut Process,
    left: &Operand,
    right: &Operand,
    dest: &Operand,
    op: impl FnOnce(f64, f64) -> f64,
) -> Result<InstructionOutcome, ExecError> {
    let left = read_float_reg(process, left)?;
    let right = read_float_reg(process, right)?;
    write_float_reg(process, dest, op(left, right))?;
    Ok(InstructionOutcome::Continue)
}

fn read_float_reg(process: &Process, operand: &Operand) -> Result<f64, ExecError> {
    match operand {
        Operand::FloatRegister(index) => process
            .get_float_reg(float_index(*index)?)
            .map_err(process_error_to_exec),
        _ => Err(ExecError::InvalidOperand("float register source")),
    }
}

fn write_float_reg(process: &mut Process, operand: &Operand, value: f64) -> Result<(), ExecError> {
    match operand {
        Operand::FloatRegister(index) => process
            .set_float_reg(float_index(*index)?, value)
            .map_err(process_error_to_exec),
        _ => Err(ExecError::InvalidOperand("float register destination")),
    }
}

fn float_index(index: u32) -> Result<u16, ExecError> {
    u16::try_from(index).map_err(|_| ExecError::InvalidOperand("float register"))
}

fn validate_term_destination(destination: &Operand) -> Result<(), ExecError> {
    match destination {
        Operand::X(_) | Operand::Y(_) => Ok(()),
        Operand::TypedRegister { register, .. } => validate_term_destination(register),
        _ => Err(ExecError::InvalidOperand("term destination")),
    }
}

fn process_error_to_exec(error: ProcessError) -> ExecError {
    match error {
        ProcessError::FloatRegisterOutOfBounds { .. } => {
            ExecError::InvalidOperand("float register")
        }
        ProcessError::InvalidStatusTransition { .. } => ExecError::Badarg,
    }
}

fn number_to_float(term: Term) -> Result<f64, ExecError> {
    if let Some(value) = term.as_small_int() {
        Ok(value as f64)
    } else {
        Float::new(term)
            .map(Float::value)
            .ok_or(ExecError::Badarith)
    }
}

fn boxed_float(process: &mut Process, value: f64) -> Result<Term, ExecError> {
    let ptr = gc::alloc(process, FLOAT_BOX_WORDS).map_err(gc_error_to_exec)?;
    let heap = core::heap_slice(ptr, FLOAT_BOX_WORDS);
    write_float(heap, value).ok_or(ExecError::Badarg)
}

fn gc_error_to_exec(error: GcError) -> ExecError {
    match error {
        GcError::HeapFull(error) => ExecError::from(error),
        GcError::InvalidObjectHeader(_) => ExecError::Badarg,
    }
}

#[cfg(test)]
mod tests;
