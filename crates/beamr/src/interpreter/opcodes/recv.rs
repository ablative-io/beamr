//! Receive marker opcode handlers.
//!
//! OTP 24+ emits these opcodes around selective receive loops. This initial
//! implementation maps the marker operations onto the existing single mailbox
//! save pointer instead of maintaining multiple independent marker slots.

use crate::error::ExecError;
use crate::interpreter::InstructionOutcome;
use crate::interpreter::opcodes::core;
use crate::loader::decode::compact::Operand;
use crate::module::Module;
use crate::process::Process;
use crate::term::Term;

/// Reserve a marker at the current mailbox save pointer and write it to `dest`.
pub fn recv_marker_reserve(
    process: &mut Process,
    dest: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    let marker = process.mailbox_mut().reserve_marker();
    let marker = i64::try_from(marker).map_err(|_| ExecError::Badarg)?;
    let marker = Term::try_small_int(marker).ok_or(ExecError::Badarg)?;
    core::write_term(process, dest, marker)?;
    Ok(InstructionOutcome::Continue)
}

/// Bind a marker to its receive discriminator.
///
/// The current stub leaves the single save-pointer state untouched. OTP 24+
/// emits a register/reference operand here for optimized receives, so the
/// operand is intentionally accepted verbatim.
pub fn recv_marker_bind(_label: &Operand) -> Result<InstructionOutcome, ExecError> {
    Ok(InstructionOutcome::Continue)
}

/// Clear the receive marker optimization hint.
pub fn recv_marker_clear(
    process: &mut Process,
    _marker: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    process.mailbox_mut().reset_save_pointer();
    Ok(InstructionOutcome::Continue)
}

/// Restore scanning to a marker value, clamped to the current mailbox length.
pub fn recv_marker_use(
    process: &mut Process,
    module: &Module,
    marker: &Operand,
) -> Result<InstructionOutcome, ExecError> {
    let marker = marker_value(process, module, marker)?;
    process.mailbox_mut().set_save_pointer(marker);
    Ok(InstructionOutcome::Continue)
}

fn marker_value(process: &Process, module: &Module, marker: &Operand) -> Result<usize, ExecError> {
    let value = core::read_term(process, module, marker)?;
    let value = value.as_small_int().ok_or(ExecError::Badarg)?;
    usize::try_from(value).map_err(|_| ExecError::Badarg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atom::Atom;
    use crate::interpreter::opcodes::dispatch;
    use crate::loader::Instruction;
    use std::collections::HashMap;

    fn module(code: Vec<Instruction>) -> Module {
        let label_index = code
            .iter()
            .enumerate()
            .filter_map(|(ip, instruction)| match instruction {
                Instruction::Label { label } => Some((*label, ip)),
                _ => None,
            })
            .collect();
        Module {
            name: Atom::OK,
            generation: 0,
            exports: HashMap::new(),
            label_index,
            code,
            literals: Vec::new(),
            constant_pool: Default::default(),
            resolved_imports: Vec::new(),
            lambdas: Vec::new(),
            string_table: Vec::new(),
            line_info: Vec::new(),
        }
    }

    #[test]
    fn reserve_writes_current_save_pointer_marker() {
        let mut process = Process::new(1, 32);
        process
            .mailbox_mut()
            .push_owned_for_test(Term::small_int(10));
        process
            .mailbox_mut()
            .push_owned_for_test(Term::small_int(20));
        process.mailbox_mut().advance_save_pointer();

        assert_eq!(
            recv_marker_reserve(&mut process, &Operand::X(0)),
            Ok(InstructionOutcome::Continue)
        );
        assert_eq!(process.x_reg(0).as_small_int(), Some(1));
    }

    #[test]
    fn bind_validates_label_and_dispatches() {
        let module = module(vec![Instruction::Label { label: 7 }]);
        let mut process = Process::new(1, 32);
        let instruction = Instruction::RecvMarkerBind {
            marker: Operand::X(0),
            label: Operand::Label(7),
        };

        assert_eq!(
            dispatch(&mut process, &module, &instruction, 1, None),
            Ok(InstructionOutcome::Continue)
        );
    }

    #[test]
    fn clear_resets_save_pointer() {
        let mut process = Process::new(1, 32);
        process
            .mailbox_mut()
            .push_owned_for_test(Term::small_int(10));
        process.mailbox_mut().advance_save_pointer();

        assert_eq!(
            recv_marker_clear(&mut process, &Operand::X(0)),
            Ok(InstructionOutcome::Continue)
        );
        assert_eq!(
            process.mailbox_mut().current_message(),
            Some(Term::small_int(10))
        );
    }

    #[test]
    fn use_restores_marker_and_clamps_to_current_mailbox() {
        let module = module(Vec::new());
        let mut process = Process::new(1, 32);
        process
            .mailbox_mut()
            .push_owned_for_test(Term::small_int(10));
        process
            .mailbox_mut()
            .push_owned_for_test(Term::small_int(20));
        process.set_x_reg(0, Term::small_int(1));

        assert_eq!(
            recv_marker_use(&mut process, &module, &Operand::X(0)),
            Ok(InstructionOutcome::Continue)
        );
        assert_eq!(
            process.mailbox_mut().current_message(),
            Some(Term::small_int(20))
        );

        process.set_x_reg(0, Term::small_int(99));
        assert_eq!(
            recv_marker_use(&mut process, &module, &Operand::X(0)),
            Ok(InstructionOutcome::Continue)
        );
        assert_eq!(process.mailbox_mut().current_message(), None);
    }

    #[test]
    fn use_rejects_negative_marker() {
        let module = module(Vec::new());
        let mut process = Process::new(1, 32);
        process.set_x_reg(0, Term::small_int(-1));

        assert_eq!(
            recv_marker_use(&mut process, &module, &Operand::X(0)),
            Err(ExecError::Badarg)
        );
    }
}
