//! Control flow structures for the JIT compiler.

use crate::loader::Instruction;
use crate::loader::decode::{BifOp, MapOp};
use cranelift_frontend::FunctionBuilder;
use std::collections::{HashMap, HashSet};

use super::compiler::JitError;
use super::ir_arithmetic::{ArithmeticOp, ParsedBif};
use super::ir_common::{
    ensure_known_label, validate_label_operand, validate_read_operand, validate_write_operand,
};
use super::ir_control_validation::{
    validate_float_register_operand, validate_fmove_operands, validate_import_operand,
    validate_supported_type_test,
};
use super::ir_guards::{
    immediate_raw_term, immediate_usize, parse_select_pairs, validate_tag_atom,
};
use super::ir_map::{
    parse_get_map_elements_operands, parse_has_map_fields_operands, parse_put_map_operands,
};

/// Where an `Instruction` variant sits in the demand-JIT coverage tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Coverage {
    /// Admitted end-to-end: the pre-pass accepts it and a dispatch lowering exists.
    Supported,
    /// Rejected at this head but owned by a named later wave; moved into
    /// `Supported` only by editing this table.
    RejectedIncremental { reason: &'static str },
    /// Rejected by design; never lowered by any wave.
    RejectedInherent { reason: &'static str },
}

/// THE coverage table of record for whole-function JIT admission.
///
/// This match is the single source of truth for which `Instruction` variants the
/// demand-JIT tier admits, and it is EXHAUSTIVE WITH NO WILDCARD ARM: adding an
/// `Instruction` variant breaks compilation here until it is classified — the
/// wall the pre-pass and dispatch catch-alls never gave us. Those two catch-alls
/// keep their `UnsupportedOpcode` returns, but this table is the authority they
/// must agree with (enforced by the 75-variant consistency walk in the compiler
/// tests). JIT-002 and JIT-003 move variants across the tier ONLY by editing
/// this function.
pub(crate) fn coverage(instruction: &Instruction) -> Coverage {
    match instruction {
        // -- Supported: the baseline 47 (dispatch_core / dispatch_call / dispatch_data) --
        Instruction::Label { .. }
        | Instruction::Move { .. }
        | Instruction::Swap { .. }
        | Instruction::Bif { .. }
        | Instruction::TypeTest { .. }
        | Instruction::Comparison { .. }
        | Instruction::TestArity { .. }
        | Instruction::IsTaggedTuple { .. }
        | Instruction::SelectVal { .. }
        | Instruction::Jump { .. }
        | Instruction::Send
        | Instruction::LoopRec { .. }
        | Instruction::LoopRecEnd { .. }
        | Instruction::RemoveMessage
        | Instruction::Wait { .. }
        | Instruction::WaitTimeout { .. }
        | Instruction::Timeout
        | Instruction::RecvMarkerReserve { .. }
        | Instruction::RecvMarkerBind { .. }
        | Instruction::RecvMarkerClear { .. }
        | Instruction::RecvMarkerUse { .. }
        | Instruction::Try { .. }
        | Instruction::TryEnd { .. }
        | Instruction::TryCase { .. }
        | Instruction::Return
        | Instruction::CallExt { .. }
        | Instruction::CallExtOnly { .. }
        | Instruction::MakeFun { .. }
        | Instruction::CallFun { .. }
        | Instruction::Call { .. }
        | Instruction::CallOnly { .. }
        | Instruction::Apply { .. }
        | Instruction::Fmove { .. }
        | Instruction::Fconv { .. }
        | Instruction::Fadd { .. }
        | Instruction::Fsub { .. }
        | Instruction::Fmul { .. }
        | Instruction::Fdiv { .. }
        | Instruction::Fnegate { .. }
        | Instruction::PutList { .. }
        | Instruction::GetList { .. }
        | Instruction::GetHd { .. }
        | Instruction::GetTl { .. }
        | Instruction::PutTuple2 { .. }
        | Instruction::GetTupleElement { .. }
        | Instruction::BinaryOp { .. }
        | Instruction::MapOp { .. }
        // -- Supported: JIT-002 R1 structural frame set (+8) --
        | Instruction::Line { .. }
        | Instruction::Allocate { .. }
        | Instruction::AllocateHeap { .. }
        | Instruction::AllocateZero { .. }
        | Instruction::Deallocate { .. }
        | Instruction::TestHeap { .. }
        | Instruction::InitYregs { .. }
        | Instruction::Trim { .. }
        // -- Supported: JIT-002 R2 tail calls (+3) --
        | Instruction::CallLast { .. }
        | Instruction::CallExtLast { .. }
        | Instruction::ApplyLast { .. }
        // -- Supported: JIT-002 R3 (+1) --
        | Instruction::CallFun2 { .. } => Coverage::Supported,

        // -- RejectedIncremental: wave 2 (the arc's next brief) --
        Instruction::SelectTupleArity { .. } => Coverage::RejectedIncremental {
            reason: "wave 2: SelectTupleArity (sibling of SelectVal)",
        },
        Instruction::Catch { .. }
        | Instruction::CatchEnd { .. }
        | Instruction::TryCaseEnd { .. }
        | Instruction::Raise { .. }
        | Instruction::RawRaise
        | Instruction::BuildStacktrace => Coverage::RejectedIncremental {
            reason: "wave 2: exception machinery / error terminals",
        },
        Instruction::Badmatch { .. }
        | Instruction::Badrecord { .. }
        | Instruction::CaseEnd { .. }
        | Instruction::IfEnd => Coverage::RejectedIncremental {
            reason: "wave 2: error-raising terminals",
        },
        Instruction::UpdateRecord { .. } => Coverage::RejectedIncremental {
            reason: "wave 2: UpdateRecord (record update over tuples)",
        },

        // -- RejectedInherent: rejected by design; no lowering attempts --
        Instruction::FuncInfo { .. } => Coverage::RejectedInherent {
            reason: "structural prelude, stripped by the AOT/edge slicer",
        },
        Instruction::OnLoad => Coverage::RejectedInherent {
            reason: "module-lifecycle pseudo-op, not steady-state execution",
        },
        Instruction::NifStart => Coverage::RejectedInherent {
            reason: "module-lifecycle pseudo-op, not interpreter-dispatched",
        },
        Instruction::Generic { .. } => Coverage::RejectedInherent {
            reason: "unknown-by-construction loader escape hatch",
        },
    }
}

pub(crate) struct TranslationPlan {
    pub(crate) labels: HashMap<u32, usize>,
    pub(crate) block_starts: HashSet<usize>,
}

impl TranslationPlan {
    pub(crate) fn new(instructions: &[Instruction]) -> Result<Self, JitError> {
        if instructions.is_empty() {
            return Err(JitError::EmptyFunction);
        }

        let mut labels = HashMap::new();
        let mut block_starts = HashSet::from([0, instructions.len()]);
        for (index, instruction) in instructions.iter().enumerate() {
            match instruction {
                Instruction::Label { label } => {
                    labels.insert(*label, index);
                    block_starts.insert(index);
                }
                Instruction::Return => {
                    block_starts.insert(index + 1);
                }
                // Debug line marker: skipped in both the block scan and dispatch.
                Instruction::Line { .. } => {}
                // Stack-frame ops. Each emits a frame-management helper guard that
                // deopts on failure, so the next instruction starts a fresh block.
                Instruction::Allocate { .. }
                | Instruction::AllocateHeap { .. }
                | Instruction::AllocateZero { .. }
                | Instruction::Deallocate { .. }
                | Instruction::TestHeap { .. }
                | Instruction::Trim { .. } => {
                    block_starts.insert(index + 1);
                }
                // NIL-initialize named Y registers (branchless Y writes).
                Instruction::InitYregs { registers } => {
                    let crate::loader::decode::Operand::List(registers) = registers else {
                        return Err(JitError::UnsupportedOperand {
                            operand: format!(
                                "init_yregs expected a register list, got {registers:?}"
                            ),
                        });
                    };
                    for register in registers {
                        validate_write_operand(register)?;
                    }
                }
                Instruction::Move {
                    source,
                    destination,
                } => {
                    validate_read_operand(source)?;
                    validate_write_operand(destination)?;
                }
                Instruction::Swap { left, right } => {
                    validate_read_operand(left)?;
                    validate_read_operand(right)?;
                    validate_write_operand(left)?;
                    validate_write_operand(right)?;
                }
                Instruction::TypeTest { op, fail, value } => {
                    validate_supported_type_test(*op)?;
                    validate_label_operand(fail)?;
                    validate_read_operand(value)?;
                    block_starts.insert(index + 1);
                }
                Instruction::PutList {
                    head,
                    tail,
                    destination,
                } => {
                    validate_read_operand(head)?;
                    validate_read_operand(tail)?;
                    validate_write_operand(destination)?;
                    block_starts.insert(index + 1);
                }
                Instruction::GetList { source, head, tail } => {
                    validate_read_operand(source)?;
                    validate_write_operand(head)?;
                    validate_write_operand(tail)?;
                }
                Instruction::GetHd {
                    source,
                    destination,
                }
                | Instruction::GetTl {
                    source,
                    destination,
                } => {
                    validate_read_operand(source)?;
                    validate_write_operand(destination)?;
                }
                Instruction::PutTuple2 {
                    destination,
                    elements,
                } => {
                    validate_write_operand(destination)?;
                    let crate::loader::decode::Operand::List(elements) = elements else {
                        return Err(JitError::UnsupportedOperand {
                            operand: format!(
                                "put_tuple2 elements must be a list, got {elements:?}"
                            ),
                        });
                    };
                    for element in elements {
                        validate_read_operand(element)?;
                    }
                    block_starts.insert(index + 1);
                }
                Instruction::GetTupleElement {
                    source,
                    index,
                    destination,
                } => {
                    validate_read_operand(source)?;
                    let _ = immediate_usize(index, "get_tuple_element index")?;
                    validate_write_operand(destination)?;
                }
                Instruction::Comparison {
                    fail, left, right, ..
                } => {
                    validate_label_operand(fail)?;
                    validate_read_operand(left)?;
                    validate_read_operand(right)?;
                    block_starts.insert(index + 1);
                }
                Instruction::TestArity { fail, tuple, arity } => {
                    validate_label_operand(fail)?;
                    validate_read_operand(tuple)?;
                    let _ = immediate_usize(arity, "test_arity arity")?;
                    block_starts.insert(index + 1);
                }
                Instruction::IsTaggedTuple {
                    fail,
                    value,
                    arity,
                    tag,
                } => {
                    validate_label_operand(fail)?;
                    validate_read_operand(value)?;
                    let _ = immediate_usize(arity, "is_tagged_tuple arity")?;
                    validate_tag_atom(tag)?;
                    block_starts.insert(index + 1);
                }
                Instruction::SelectVal { value, fail, list } => {
                    validate_read_operand(value)?;
                    validate_label_operand(fail)?;
                    for (candidate, target) in parse_select_pairs(list)? {
                        let _ = immediate_raw_term(candidate)?;
                        validate_label_operand(target)?;
                    }
                    block_starts.insert(index + 1);
                }
                Instruction::Jump { target } => {
                    validate_label_operand(target)?;
                    block_starts.insert(index + 1);
                }
                Instruction::Try { destination, label } => {
                    validate_write_operand(destination)?;
                    validate_label_operand(label)?;
                    block_starts.insert(index + 1);
                }
                Instruction::TryEnd { source } | Instruction::TryCase { source } => {
                    validate_write_operand(source)?;
                    block_starts.insert(index + 1);
                }
                Instruction::Call { label, .. }
                | Instruction::CallOnly { label, .. }
                | Instruction::CallLast { label, .. } => {
                    validate_label_operand(label)?;
                    block_starts.insert(index + 1);
                }
                Instruction::CallExt { import, .. }
                | Instruction::CallExtOnly { import, .. }
                | Instruction::CallExtLast { import, .. } => {
                    validate_import_operand(import)?;
                    block_starts.insert(index + 1);
                }
                Instruction::Bif { op, operands } => {
                    let parsed = ParsedBif::parse(*op, operands)?;
                    let _ = ArithmeticOp::from_import(parsed.import)?;
                    validate_label_operand(parsed.fail)?;
                    validate_read_operand(parsed.left)?;
                    validate_read_operand(parsed.right)?;
                    validate_write_operand(parsed.destination)?;
                    block_starts.insert(index + 1);
                }
                Instruction::MakeFun { .. } => {
                    block_starts.insert(index + 1);
                }
                Instruction::Fmove { source, dest } => {
                    validate_fmove_operands(source, dest)?;
                    block_starts.insert(index + 1);
                }
                Instruction::Fconv { source, dest } => {
                    validate_read_operand(source)?;
                    validate_float_register_operand(dest, "fconv destination")?;
                    block_starts.insert(index + 1);
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
                } => {
                    validate_label_operand(fail)?;
                    validate_float_register_operand(left, "float arithmetic left")?;
                    validate_float_register_operand(right, "float arithmetic right")?;
                    validate_float_register_operand(dest, "float arithmetic destination")?;
                    block_starts.insert(index + 1);
                }
                Instruction::Fnegate { fail, source, dest } => {
                    validate_label_operand(fail)?;
                    validate_float_register_operand(source, "fnegate source")?;
                    validate_float_register_operand(dest, "fnegate destination")?;
                    block_starts.insert(index + 1);
                }
                Instruction::Apply { .. }
                | Instruction::ApplyLast { .. }
                | Instruction::CallFun { .. }
                // CallFun2's lowering already exists at dispatch_call.rs; admitting
                // it here makes that dead lowering reachable (JIT-002 R3).
                | Instruction::CallFun2 { .. } => {
                    block_starts.insert(index + 1);
                }
                Instruction::BinaryOp { .. } => {
                    block_starts.insert(index + 1);
                }
                Instruction::MapOp { op, operands } => {
                    match op {
                        MapOp::PutMapAssoc | MapOp::PutMapExact => {
                            let (fail, parsed) = parse_put_map_operands(operands)?;
                            validate_label_operand(fail)?;
                            validate_read_operand(parsed.source)?;
                            validate_write_operand(parsed.destination)?;
                            for operand in parsed.pairs {
                                validate_read_operand(operand)?;
                            }
                        }
                        MapOp::GetMapElements => {
                            let (fail, parsed) = parse_get_map_elements_operands(operands)?;
                            validate_label_operand(fail)?;
                            validate_read_operand(parsed.source)?;
                            for pair in parsed.pairs.chunks_exact(2) {
                                validate_read_operand(&pair[0])?;
                                validate_write_operand(&pair[1])?;
                            }
                        }
                        MapOp::HasMapFields => {
                            let (fail, parsed) = parse_has_map_fields_operands(operands)?;
                            validate_label_operand(fail)?;
                            validate_read_operand(parsed.source)?;
                            for key in parsed.keys {
                                validate_read_operand(key)?;
                            }
                        }
                    }
                    block_starts.insert(index + 1);
                }
                Instruction::Send | Instruction::RemoveMessage | Instruction::Timeout => {
                    block_starts.insert(index + 1);
                }
                Instruction::LoopRec { fail, destination } => {
                    validate_label_operand(fail)?;
                    validate_write_operand(destination)?;
                    block_starts.insert(index + 1);
                }
                Instruction::LoopRecEnd { fail } | Instruction::Wait { fail } => {
                    validate_label_operand(fail)?;
                    block_starts.insert(index + 1);
                }
                Instruction::WaitTimeout { fail, timeout } => {
                    validate_label_operand(fail)?;
                    validate_read_operand(timeout)?;
                    block_starts.insert(index + 1);
                }
                Instruction::RecvMarkerReserve { dest } => {
                    validate_write_operand(dest)?;
                    block_starts.insert(index + 1);
                }
                Instruction::RecvMarkerBind { marker, reference } => {
                    validate_read_operand(marker)?;
                    validate_read_operand(reference)?;
                    block_starts.insert(index + 1);
                }
                Instruction::RecvMarkerClear { marker } | Instruction::RecvMarkerUse { marker } => {
                    validate_read_operand(marker)?;
                    block_starts.insert(index + 1);
                }
                other => {
                    // The pre-pass rejects here only variants the coverage table
                    // marks non-Supported; a divergence is a coverage-table bug.
                    debug_assert_ne!(
                        coverage(other),
                        Coverage::Supported,
                        "pre-pass rejected a variant the coverage table marks Supported: {}",
                        opcode_name(other)
                    );
                    return Err(JitError::UnsupportedOpcode {
                        opcode: opcode_name(other),
                    });
                }
            }
        }

        for instruction in instructions {
            match instruction {
                Instruction::TypeTest { fail, .. }
                | Instruction::Comparison { fail, .. }
                | Instruction::TestArity { fail, .. }
                | Instruction::IsTaggedTuple { fail, .. } => ensure_known_label(&labels, fail)?,
                Instruction::SelectVal { fail, list, .. } => {
                    ensure_known_label(&labels, fail)?;
                    for (_, target) in parse_select_pairs(list)? {
                        ensure_known_label(&labels, target)?;
                    }
                }
                Instruction::Jump { target }
                | Instruction::Try { label: target, .. }
                | Instruction::Call { label: target, .. }
                | Instruction::CallOnly { label: target, .. }
                | Instruction::CallLast { label: target, .. } => {
                    ensure_known_label(&labels, target)?
                }
                Instruction::Bif { op, operands } => {
                    if matches!(op, BifOp::Bif2 | BifOp::GcBif2) {
                        let parsed = ParsedBif::parse(*op, operands)?;
                        ensure_known_label(&labels, parsed.fail)?;
                    }
                }
                Instruction::Fadd { fail, .. }
                | Instruction::Fsub { fail, .. }
                | Instruction::Fmul { fail, .. }
                | Instruction::Fdiv { fail, .. }
                | Instruction::Fnegate { fail, .. } => {
                    ensure_known_label(&labels, fail)?;
                }
                Instruction::LoopRec { fail, .. }
                | Instruction::LoopRecEnd { fail }
                | Instruction::Wait { fail }
                | Instruction::WaitTimeout { fail, .. } => {
                    ensure_known_label(&labels, fail)?;
                }
                Instruction::MapOp { op, operands } => {
                    let fail = match op {
                        MapOp::PutMapAssoc | MapOp::PutMapExact => {
                            parse_put_map_operands(operands)?.0
                        }
                        MapOp::GetMapElements => parse_get_map_elements_operands(operands)?.0,
                        MapOp::HasMapFields => parse_has_map_fields_operands(operands)?.0,
                    };
                    ensure_known_label(&labels, fail)?;
                }
                _ => {}
            }
        }

        Ok(Self {
            labels,
            block_starts,
        })
    }
}

pub(crate) struct BlockMap {
    blocks_by_index: Vec<cranelift_codegen::ir::Block>,
    label_blocks: HashMap<u32, cranelift_codegen::ir::Block>,
    pub(crate) entry: cranelift_codegen::ir::Block,
    pub(crate) deopt: cranelift_codegen::ir::Block,
    pub(crate) exception_block: cranelift_codegen::ir::Block,
    pub(crate) yield_block: cranelift_codegen::ir::Block,
}

impl BlockMap {
    pub(crate) fn new(
        builder: &mut FunctionBuilder<'_>,
        instructions: &[Instruction],
        plan: &TranslationPlan,
    ) -> Self {
        let mut blocks_by_index = Vec::with_capacity(instructions.len() + 1);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        let mut current = builder.create_block();
        for index in 0..=instructions.len() {
            if index > 0 && plan.block_starts.contains(&index) {
                current = builder.create_block();
            }
            blocks_by_index.push(current);
        }

        let mut label_blocks = HashMap::new();
        for (label, index) in &plan.labels {
            label_blocks.insert(*label, blocks_by_index[*index]);
        }

        Self {
            entry,
            blocks_by_index,
            label_blocks,
            deopt: builder.create_block(),
            exception_block: builder.create_block(),
            yield_block: builder.create_block(),
        }
    }

    pub(crate) fn block_for_instruction(&self, index: usize) -> cranelift_codegen::ir::Block {
        self.blocks_by_index[index]
    }

    pub(crate) fn block_after(&self, index: usize) -> cranelift_codegen::ir::Block {
        self.blocks_by_index[index + 1]
    }

    pub(crate) fn exit_block(&self) -> cranelift_codegen::ir::Block {
        self.blocks_by_index[self.blocks_by_index.len() - 1]
    }

    pub(crate) fn label_block(&self, label: u32) -> Result<cranelift_codegen::ir::Block, JitError> {
        self.label_blocks
            .get(&label)
            .copied()
            .ok_or(JitError::UnknownLabel { label })
    }
}

pub(crate) fn opcode_name(instruction: &Instruction) -> String {
    match instruction {
        Instruction::Generic { opcode, name, .. } => format!("{name} ({opcode})"),
        other => format!("{other:?}"),
    }
}
