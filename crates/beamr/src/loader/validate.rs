//! Instruction operand validation.
//!
//! After decoding, validates that instruction operands are well-formed:
//! register indices within range, label targets exist in the code,
//! arities match function signatures, and atom indices resolve in the
//! atom table. Invalid instructions produce actionable error messages
//! naming the instruction and the specific operand that failed.

use std::collections::{HashMap, HashSet};

use crate::error::LoadError;
use crate::loader::decode::Operand;
use crate::loader::{Instruction, ParsedModule};
use crate::module::ResolvedImport;

/// Validates decoded instructions and import operands before registration.
pub fn validate_module(
    parsed: &ParsedModule,
    resolved_imports: &[ResolvedImport],
) -> Result<(), LoadError> {
    let labels = collect_labels(&parsed.instructions);
    let functions = collect_function_arities(&parsed.instructions);

    for export in &parsed.exports {
        if !labels.contains(&export.label) {
            return Err(validation_error(
                0,
                format!("export label {} does not exist", export.label),
            ));
        }
    }

    let mut current_frame_size: Option<u32> = None;
    for (index, instruction) in parsed.instructions.iter().enumerate() {
        validate_instruction_operands(index, instruction, current_frame_size)?;
        validate_control_flow(
            index,
            instruction,
            parsed,
            resolved_imports,
            &labels,
            &functions,
        )?;
        update_frame_size(index, instruction, &mut current_frame_size)?;
    }

    Ok(())
}

fn collect_labels(instructions: &[Instruction]) -> HashSet<u32> {
    instructions
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::Label { label } => Some(*label),
            _ => None,
        })
        .collect()
}

fn collect_function_arities(instructions: &[Instruction]) -> HashMap<u32, u8> {
    let mut arities = HashMap::new();
    let mut previous_label = None;
    for instruction in instructions {
        match instruction {
            Instruction::Label { label } => previous_label = Some(*label),
            Instruction::FuncInfo { arity, .. } => {
                if let (Some(label), Some(arity)) = (previous_label, operand_to_u8(arity)) {
                    arities.insert(label, arity);
                }
            }
            _ => {}
        }
    }
    arities
}

fn validate_instruction_operands(
    instruction_index: usize,
    instruction: &Instruction,
    current_frame_size: Option<u32>,
) -> Result<(), LoadError> {
    for operand in instruction_operands(instruction) {
        validate_operand(instruction_index, operand, current_frame_size)?;
    }
    Ok(())
}

fn validate_operand(
    instruction_index: usize,
    operand: &Operand,
    current_frame_size: Option<u32>,
) -> Result<(), LoadError> {
    match operand {
        Operand::X(index) if *index >= 256 => Err(validation_error(
            instruction_index,
            format!("X register index {index} is out of range"),
        )),
        Operand::Y(index) => {
            if *index >= 256 {
                return Err(validation_error(
                    instruction_index,
                    format!("Y register index {index} is out of range"),
                ));
            }
            match current_frame_size {
                Some(frame_size) if *index < frame_size => Ok(()),
                Some(frame_size) => Err(validation_error(
                    instruction_index,
                    format!("Y register index {index} is outside frame size {frame_size}"),
                )),
                None => Ok(()),
            }
        }
        Operand::FloatRegister(index) if *index >= 16 => Err(validation_error(
            instruction_index,
            format!("float register index {index} is out of range"),
        )),
        Operand::List(operands) => {
            for nested in operands {
                validate_operand(instruction_index, nested, current_frame_size)?;
            }
            Ok(())
        }
        Operand::TypedRegister { register, .. } => {
            validate_operand(instruction_index, register, current_frame_size)
        }
        _ => Ok(()),
    }
}

fn validate_control_flow(
    instruction_index: usize,
    instruction: &Instruction,
    parsed: &ParsedModule,
    resolved_imports: &[ResolvedImport],
    labels: &HashSet<u32>,
    functions: &HashMap<u32, u8>,
) -> Result<(), LoadError> {
    match instruction {
        Instruction::Call { arity, label }
        | Instruction::CallOnly { arity, label }
        | Instruction::CallLast { arity, label, .. } => {
            let label = expect_label_operand(instruction_index, label, labels)?;
            if let Some(expected) = functions.get(&label) {
                let actual = expect_arity_operand(instruction_index, arity, "call arity")?;
                if actual != *expected {
                    return Err(validation_error(
                        instruction_index,
                        format!(
                            "call arity {actual} does not match target label {label} arity {expected}"
                        ),
                    ));
                }
            }
        }
        Instruction::CallExt { arity, import }
        | Instruction::CallExtOnly { arity, import }
        | Instruction::CallExtLast { arity, import, .. } => {
            let import_index = expect_import_index(instruction_index, import)?;
            let import_entry = parsed.imports.get(import_index).ok_or_else(|| {
                validation_error(
                    instruction_index,
                    format!("import index {import_index} is out of range"),
                )
            })?;
            if import_index >= resolved_imports.len() {
                return Err(validation_error(
                    instruction_index,
                    format!("resolved import index {import_index} is out of range"),
                ));
            }
            let actual = expect_arity_operand(instruction_index, arity, "external call arity")?;
            if actual != import_entry.arity {
                return Err(validation_error(
                    instruction_index,
                    format!(
                        "external call arity {actual} does not match import index {import_index} arity {}",
                        import_entry.arity
                    ),
                ));
            }
        }
        Instruction::TypeTest { fail, .. }
        | Instruction::Comparison { fail, .. }
        | Instruction::Fadd { fail, .. }
        | Instruction::Fsub { fail, .. }
        | Instruction::Fmul { fail, .. }
        | Instruction::Fdiv { fail, .. }
        | Instruction::Fnegate { fail, .. }
        | Instruction::TestArity { fail, .. }
        | Instruction::IsTaggedTuple { fail, .. }
        | Instruction::SelectVal { fail, .. }
        | Instruction::SelectTupleArity { fail, .. }
        | Instruction::LoopRec { fail, .. }
        | Instruction::LoopRecEnd { fail }
        | Instruction::Wait { fail }
        | Instruction::WaitTimeout { fail, .. } => {
            expect_label_operand(instruction_index, fail, labels)?;
        }
        Instruction::Jump { target } => {
            expect_label_operand(instruction_index, target, labels)?;
        }
        Instruction::Catch { label, .. } | Instruction::Try { label, .. } => {
            expect_label_operand(instruction_index, label, labels)?;
        }
        _ => {}
    }
    Ok(())
}

fn update_frame_size(
    instruction_index: usize,
    instruction: &Instruction,
    current_frame_size: &mut Option<u32>,
) -> Result<(), LoadError> {
    match instruction {
        Instruction::Allocate { stack_need, .. }
        | Instruction::AllocateHeap { stack_need, .. }
        | Instruction::AllocateZero { stack_need, .. } => {
            *current_frame_size = Some(operand_to_u32(stack_need).ok_or_else(|| {
                validation_error(
                    instruction_index,
                    format!(
                        "allocate stack frame operand {stack_need:?} is not an unsigned integer"
                    ),
                )
            })?);
        }
        Instruction::Deallocate { .. } => *current_frame_size = None,
        // Trim shrinks the live frame to its `remaining` operand (and renumbers
        // the survivors down to Y0..remaining-1). Tracked so straight-line code
        // after a trim is still checked against the true, smaller frame. When
        // the model is already unknown it stays unknown: trim states a delta,
        // not an absolute, and inventing a frame here could false-red.
        Instruction::Trim { remaining, .. } => {
            if current_frame_size.is_some() {
                *current_frame_size = operand_to_u32(remaining);
            }
        }
        // Frame knowledge is CONTROL-FLOW state, and this scan is linear. The
        // position after an instruction that never falls through (returns,
        // tail calls, unconditional jumps, raising terminators) is reachable
        // only via a label — from jump sites whose frames this scan cannot
        // know. Compilers place shared failure islands (badmatch/case_end
        // blocks) exactly there, ordered after OTHER functions' tails, so
        // checking them against the linear predecessor's frame false-reds
        // legal emission. Measured, not hypothetical: OTP 27's erlc puts
        // `index/5`'s badmatch island in gleam_stdlib's gleam@dynamic@decode
        // after a frame-1 tail, and the island names Y5 — valid under its
        // real jump-source frame of 7 (aion#64). Unknown is the only honest
        // model at these boundaries; Y checks resume at the next allocate.
        Instruction::Return
        | Instruction::CallLast { .. }
        | Instruction::CallExtLast { .. }
        | Instruction::ApplyLast { .. }
        | Instruction::Jump { .. }
        | Instruction::RawRaise
        | Instruction::Raise { .. }
        | Instruction::Badmatch { .. }
        | Instruction::Badrecord { .. }
        | Instruction::CaseEnd { .. }
        | Instruction::IfEnd
        | Instruction::TryCaseEnd { .. }
        | Instruction::LoopRecEnd { .. }
        | Instruction::Wait { .. }
        // A function boundary always invalidates the previous frame, whatever
        // the preceding shape was.
        | Instruction::FuncInfo { .. } => *current_frame_size = None,
        _ => {}
    }
    Ok(())
}

fn expect_label_operand(
    instruction_index: usize,
    operand: &Operand,
    labels: &HashSet<u32>,
) -> Result<u32, LoadError> {
    let label = match operand {
        Operand::Label(label) => *label,
        Operand::Unsigned(label) | Operand::Character(label) => {
            u32::try_from(*label).map_err(|_| {
                validation_error(
                    instruction_index,
                    format!("label operand {label} is out of range"),
                )
            })?
        }
        Operand::Integer(label) if *label >= 0 => u32::try_from(*label).map_err(|_| {
            validation_error(
                instruction_index,
                format!("label operand {label} is out of range"),
            )
        })?,
        other => {
            return Err(validation_error(
                instruction_index,
                format!("label operand {other:?} is not a label"),
            ));
        }
    };
    if label == 0 || labels.contains(&label) {
        Ok(label)
    } else {
        Err(validation_error(
            instruction_index,
            format!("label target {label} does not exist"),
        ))
    }
}

fn expect_import_index(instruction_index: usize, operand: &Operand) -> Result<usize, LoadError> {
    match operand_to_u64(operand).and_then(|value| usize::try_from(value).ok()) {
        Some(index) => Ok(index),
        None => Err(validation_error(
            instruction_index,
            format!("import index operand {operand:?} is invalid"),
        )),
    }
}

fn expect_arity_operand(
    instruction_index: usize,
    operand: &Operand,
    name: &str,
) -> Result<u8, LoadError> {
    operand_to_u8(operand).ok_or_else(|| {
        validation_error(
            instruction_index,
            format!("{name} operand {operand:?} is not a valid u8 arity"),
        )
    })
}

fn operand_to_u8(operand: &Operand) -> Option<u8> {
    operand_to_u64(operand).and_then(|value| u8::try_from(value).ok())
}

fn operand_to_u32(operand: &Operand) -> Option<u32> {
    operand_to_u64(operand).and_then(|value| u32::try_from(value).ok())
}

fn operand_to_u64(operand: &Operand) -> Option<u64> {
    match operand {
        Operand::Unsigned(value) => Some(*value),
        Operand::Integer(value) if *value >= 0 => u64::try_from(*value).ok(),
        _ => None,
    }
}

fn validation_error(instruction_index: usize, message: String) -> LoadError {
    LoadError::ValidationError(format!("instruction {instruction_index}: {message}"))
}

fn instruction_operands(instruction: &Instruction) -> Vec<&Operand> {
    match instruction {
        Instruction::Label { .. }
        | Instruction::Return
        | Instruction::Send
        | Instruction::RemoveMessage
        | Instruction::Timeout
        | Instruction::IfEnd
        | Instruction::RawRaise
        | Instruction::OnLoad
        | Instruction::BuildStacktrace
        | Instruction::NifStart => Vec::new(),
        Instruction::FuncInfo {
            module,
            function,
            arity,
        } => vec![module, function, arity],
        Instruction::Move {
            source,
            destination,
        } => vec![source, destination],
        Instruction::Fmove { source, dest } | Instruction::Fconv { source, dest } => {
            vec![source, dest]
        }
        Instruction::Fadd {
            fail,
            left,
            right,
            dest,
        }
        | Instruction::Fsub {
            fail,
            left,
            right,
            dest,
        }
        | Instruction::Fmul {
            fail,
            left,
            right,
            dest,
        }
        | Instruction::Fdiv {
            fail,
            left,
            right,
            dest,
        } => vec![fail, left, right, dest],
        Instruction::Fnegate { fail, source, dest } => vec![fail, source, dest],
        Instruction::Call { arity, label } | Instruction::CallOnly { arity, label } => {
            vec![arity, label]
        }
        Instruction::CallExt { arity, import } | Instruction::CallExtOnly { arity, import } => {
            vec![arity, import]
        }
        Instruction::CallLast {
            arity,
            label,
            deallocate,
        } => vec![arity, label, deallocate],
        Instruction::CallExtLast {
            arity,
            import,
            deallocate,
        } => vec![arity, import, deallocate],
        Instruction::Allocate { stack_need, live }
        | Instruction::AllocateZero { stack_need, live } => {
            vec![stack_need, live]
        }
        Instruction::AllocateHeap {
            stack_need,
            heap_need,
            live,
        } => vec![stack_need, heap_need, live],
        Instruction::Deallocate { words } => vec![words],
        Instruction::TestHeap { heap_need, live } => vec![heap_need, live],
        Instruction::PutList {
            head,
            tail,
            destination,
        } => vec![head, tail, destination],
        Instruction::PutTuple2 {
            destination,
            elements,
        } => vec![destination, elements],
        Instruction::GetTupleElement {
            source,
            index,
            destination,
        } => vec![source, index, destination],
        Instruction::GetList { source, head, tail } => vec![source, head, tail],
        Instruction::GetHd {
            source,
            destination,
        }
        | Instruction::GetTl {
            source,
            destination,
        } => vec![source, destination],
        Instruction::TypeTest { fail, value, .. } => vec![fail, value],
        Instruction::Comparison {
            fail, left, right, ..
        } => vec![fail, left, right],
        Instruction::TestArity { fail, tuple, arity } => vec![fail, tuple, arity],
        Instruction::IsTaggedTuple {
            fail,
            value,
            arity,
            tag,
        } => vec![fail, value, arity, tag],
        Instruction::SelectVal { value, fail, list }
        | Instruction::SelectTupleArity { value, fail, list } => vec![value, fail, list],
        Instruction::Jump { target } => vec![target],
        Instruction::Bif { operands, .. }
        | Instruction::BinaryOp { operands, .. }
        | Instruction::MapOp { operands, .. }
        | Instruction::MakeFun { operands }
        | Instruction::Generic { operands, .. }
        | Instruction::UpdateRecord { operands } => operands.iter().collect(),
        Instruction::LoopRec { fail, destination } => vec![fail, destination],
        Instruction::LoopRecEnd { fail } | Instruction::Wait { fail } => vec![fail],
        Instruction::WaitTimeout { fail, timeout } => vec![fail, timeout],
        Instruction::RecvMarkerReserve { dest } => vec![dest],
        Instruction::RecvMarkerBind { marker, reference } => vec![marker, reference],
        Instruction::RecvMarkerClear { marker } | Instruction::RecvMarkerUse { marker } => {
            vec![marker]
        }
        Instruction::Catch { destination, label } | Instruction::Try { destination, label } => {
            vec![destination, label]
        }
        Instruction::CatchEnd { source }
        | Instruction::TryEnd { source }
        | Instruction::TryCase { source }
        | Instruction::TryCaseEnd { source }
        | Instruction::Badmatch { value: source }
        | Instruction::Badrecord { value: source }
        | Instruction::CaseEnd { value: source }
        | Instruction::Line { index: source } => vec![source],
        Instruction::Raise { stacktrace, reason } => vec![stacktrace, reason],
        Instruction::Trim { words, remaining } => vec![words, remaining],
        Instruction::Swap { left, right } => vec![left, right],
        Instruction::InitYregs { registers } => vec![registers],
        Instruction::CallFun { arity } | Instruction::Apply { arity } => vec![arity],
        Instruction::CallFun2 {
            function,
            arity,
            destination,
        } => vec![function, arity, destination],
        Instruction::ApplyLast { arity, deallocate } => vec![arity, deallocate],
    }
}

#[cfg(test)]
mod tests {
    use super::validate_module;
    use crate::atom::AtomTable;
    use crate::loader::decode::{ExportEntry, Operand};
    use crate::loader::{Instruction, ParsedModule};

    fn parsed(instructions: Vec<Instruction>) -> ParsedModule {
        let atoms = AtomTable::new();
        ParsedModule {
            name: atoms.intern("sample"),
            atoms: Vec::new(),
            instructions,
            imports: Vec::new(),
            exports: vec![ExportEntry {
                function: atoms.intern("main"),
                arity: 0,
                label: 1,
            }],
            lambdas: Vec::new(),
            literals: Vec::new(),
            string_table: Vec::new(),
            line_info: Vec::new(),
            type_chunk: None,
        }
    }

    #[test]
    fn valid_operands_pass_validation() {
        let module = parsed(vec![
            Instruction::Label { label: 1 },
            Instruction::FuncInfo {
                module: Operand::Atom(None),
                function: Operand::Atom(None),
                arity: Operand::Integer(0),
            },
            Instruction::Allocate {
                stack_need: Operand::Integer(1),
                live: Operand::Integer(0),
            },
            Instruction::Move {
                source: Operand::X(0),
                destination: Operand::Y(0),
            },
            Instruction::Deallocate {
                words: Operand::Integer(1),
            },
            Instruction::Return,
        ]);

        assert!(validate_module(&module, &[]).is_ok());
    }

    #[test]
    fn x_register_out_of_range_is_validation_error() {
        let module = parsed(vec![
            Instruction::Label { label: 1 },
            Instruction::Move {
                source: Operand::X(999),
                destination: Operand::X(0),
            },
        ]);

        let message = validate_module(&module, &[])
            .expect_err("invalid X register")
            .to_string();

        assert!(message.contains("instruction 1"));
        assert!(message.contains("999"));
    }

    /// The aion#64 shape, minimized: a shared failure island (label +
    /// badmatch) placed after a DIFFERENT function's tail call. The island is
    /// reachable only by jump from a frame-7 region, so its Y5 is legal; the
    /// linear predecessor's frame-1 must not be held against it. OTP 27's
    /// erlc emits exactly this in gleam_stdlib's gleam@dynamic@decode.
    #[test]
    fn shared_failure_island_after_foreign_tail_is_not_checked_against_stale_frame() {
        let module = parsed(vec![
            Instruction::Label { label: 1 },
            Instruction::Allocate {
                stack_need: Operand::Integer(1),
                live: Operand::Integer(0),
            },
            Instruction::Move {
                source: Operand::X(0),
                destination: Operand::Y(0),
            },
            Instruction::CallLast {
                arity: Operand::Integer(1),
                label: Operand::Label(1),
                deallocate: Operand::Integer(1),
            },
            Instruction::Label { label: 2 },
            Instruction::Badmatch {
                value: Operand::Y(5),
            },
        ]);

        assert!(validate_module(&module, &[]).is_ok());
    }

    /// The reset is a control-flow boundary, not a blanket waiver: inside a
    /// straight-line allocate region an out-of-frame Y stays a red.
    #[test]
    fn y_register_beyond_frame_in_straight_line_code_stays_red() {
        let module = parsed(vec![
            Instruction::Label { label: 1 },
            Instruction::Allocate {
                stack_need: Operand::Integer(1),
                live: Operand::Integer(0),
            },
            Instruction::Move {
                source: Operand::X(0),
                destination: Operand::Y(5),
            },
        ]);

        let message = validate_module(&module, &[])
            .expect_err("straight-line Y5 under frame 1")
            .to_string();

        assert!(message.contains("outside frame size 1"));
    }

    /// Trim shrinks the checked frame: survivors stay valid, the trimmed
    /// range reds.
    #[test]
    fn trim_shrinks_the_checked_frame() {
        let base = |tail: Instruction| {
            parsed(vec![
                Instruction::Label { label: 1 },
                Instruction::Allocate {
                    stack_need: Operand::Integer(7),
                    live: Operand::Integer(0),
                },
                Instruction::Move {
                    source: Operand::X(0),
                    destination: Operand::Y(6),
                },
                Instruction::Trim {
                    words: Operand::Integer(4),
                    remaining: Operand::Integer(3),
                },
                tail,
            ])
        };

        assert!(
            validate_module(
                &base(Instruction::Move {
                    source: Operand::X(0),
                    destination: Operand::Y(2),
                }),
                &[]
            )
            .is_ok(),
            "Y2 survives a trim to 3"
        );

        let message = validate_module(
            &base(Instruction::Move {
                source: Operand::X(0),
                destination: Operand::Y(3),
            }),
            &[],
        )
        .expect_err("Y3 after trim to 3")
        .to_string();
        assert!(message.contains("outside frame size 3"));
    }

    #[test]
    fn missing_label_is_validation_error() {
        let module = parsed(vec![
            Instruction::Label { label: 1 },
            Instruction::Jump {
                target: Operand::Label(999),
            },
        ]);

        let message = validate_module(&module, &[])
            .expect_err("missing label")
            .to_string();

        assert!(message.contains("instruction 1"));
        assert!(message.contains("label target 999"));
    }

    #[test]
    fn float_register_out_of_range_is_validation_error() {
        let module = parsed(vec![
            Instruction::Label { label: 1 },
            Instruction::Fadd {
                fail: Operand::Label(1),
                left: Operand::FloatRegister(0),
                right: Operand::FloatRegister(16),
                dest: Operand::FloatRegister(2),
            },
        ]);

        let message = validate_module(&module, &[])
            .expect_err("invalid float register")
            .to_string();

        assert!(message.contains("instruction 1"));
        assert!(message.contains("float register index 16"));
    }

    #[test]
    fn float_fail_label_is_validation_error_when_missing() {
        let module = parsed(vec![
            Instruction::Label { label: 1 },
            Instruction::Fnegate {
                fail: Operand::Label(999),
                source: Operand::FloatRegister(0),
                dest: Operand::FloatRegister(1),
            },
        ]);

        let message = validate_module(&module, &[])
            .expect_err("invalid float fail label")
            .to_string();

        assert!(message.contains("instruction 1"));
        assert!(message.contains("label target 999"));
    }
}
