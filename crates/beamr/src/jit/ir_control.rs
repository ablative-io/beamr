//! Control flow structures for the JIT compiler.

use crate::loader::Instruction;
use crate::loader::decode::{BifOp, MapOp};
use cranelift_frontend::FunctionBuilder;
use std::collections::{HashMap, HashSet};

use super::compiler::JitError;
use super::coverage::{
    Coverage, coverage, is_no_fail_label, is_observable_side_effect, is_runtime_deopt_capable,
};
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
                // Plain Call lowers to a bare in-slice jump; the *Only/*Last forms
                // are inherently tail. A body-position Call would drop everything
                // after it, so it is admitted only immediately before a Return.
                Instruction::Call { label, .. } => {
                    require_tail_position(instruction, instructions.get(index + 1))?;
                    validate_label_operand(label)?;
                    block_starts.insert(index + 1);
                }
                Instruction::CallOnly { label, .. } | Instruction::CallLast { label, .. } => {
                    validate_label_operand(label)?;
                    block_starts.insert(index + 1);
                }
                // CallExt returns the callee's result and terminates the compiled
                // function (no body-call model), so it is admitted only in tail
                // position; CallExtOnly/CallExtLast are inherently tail.
                Instruction::CallExt { import, .. } => {
                    require_tail_position(instruction, instructions.get(index + 1))?;
                    validate_import_operand(import)?;
                    block_starts.insert(index + 1);
                }
                Instruction::CallExtOnly { import, .. }
                | Instruction::CallExtLast { import, .. } => {
                    validate_import_operand(import)?;
                    block_starts.insert(index + 1);
                }
                Instruction::Bif { op, operands } => {
                    let parsed = ParsedBif::parse(*op, operands)?;
                    let _ = ArithmeticOp::from_import(parsed.import)?;
                    // {f,0} (no local handler) routes to deopt in the lowering; a
                    // real fail label is validated as an in-slice target. The
                    // deopt-after-side-effect hazard (both the {f,0} route and the
                    // typed-overflow route target the deopt block) is handled
                    // uniformly by the pre-pass guard above through
                    // `is_runtime_deopt_capable(Bif)`.
                    if !is_no_fail_label(parsed.fail) {
                        validate_label_operand(parsed.fail)?;
                    }
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
                // Apply/CallFun/CallFun2 are helper-return calls that terminate the
                // compiled function, so they too are admitted only in tail position.
                // CallFun2's lowering already exists at dispatch_call.rs; admitting
                // it here (in tail position) makes that dead lowering reachable
                // (JIT-002 R3). ApplyLast is inherently tail.
                Instruction::Apply { .. }
                | Instruction::CallFun { .. }
                | Instruction::CallFun2 { .. } => {
                    require_tail_position(instruction, instructions.get(index + 1))?;
                    block_starts.insert(index + 1);
                }
                Instruction::ApplyLast { .. } => {
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
                // Function-clause landing pad (LEG 1c A2): a DEOPT terminal reached
                // only via a dispatch fail edge (a select_val/test fail targeting
                // the func_info prelude label). Normal calls enter at the label
                // AFTER it; when reached, it deopts and the restarted interpreter
                // raises error:function_clause.
                Instruction::FuncInfo { .. } => {
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
                        // {f,0} is the no-fail sentinel (routes to deopt), not a
                        // real label — skip the known-label check for it.
                        if !is_no_fail_label(parsed.fail) {
                            ensure_known_label(&labels, parsed.fail)?;
                        }
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

        reject_deopt_after_side_effect(instructions, &labels)?;

        Ok(Self {
            labels,
            block_starts,
        })
    }
}

/// Successor instruction indices of `instructions[index]` in the slice's control
/// flow — the edges the deopt-after-side-effect dataflow propagates along.
///
/// The critical distinction is FALL-THROUGH vs no-fall-through: an instruction
/// that leaves the function (Return, the helper-return / tail call forms, the
/// FuncInfo deopt terminal, the recv-marker deopt terminals) has NO in-function
/// successor, so an effect on ITS path never reaches a sibling clause — that is
/// what a linear scan got wrong. Explicit branch/jump/fail targets are resolved
/// through the already-validated `labels`. Deopt edges are NOT successors: a
/// deopt exits native to the interpreter, it does not continue in-slice.
fn control_flow_successors(
    index: usize,
    instructions: &[Instruction],
    labels: &HashMap<u32, usize>,
) -> Vec<usize> {
    use crate::loader::decode::Operand;
    let len = instructions.len();
    let fall_through = if index + 1 < len {
        Some(index + 1)
    } else {
        None
    };
    let resolve = |operand: &Operand| -> Option<usize> {
        match operand {
            Operand::Label(label) => labels.get(label).copied(),
            _ => None,
        }
    };
    let mut succ = Vec::new();
    match &instructions[index] {
        // Leaves the function: no in-slice successor. (Helper-return / tail calls
        // return the callee result; FuncInfo and the recv-markers deopt; Return
        // returns.) An effect before these never reaches a later slice instruction
        // by fall-through — only a real branch can, and that branch is its own
        // edge below.
        Instruction::Return
        | Instruction::CallExt { .. }
        | Instruction::CallExtOnly { .. }
        | Instruction::CallExtLast { .. }
        | Instruction::Apply { .. }
        | Instruction::ApplyLast { .. }
        | Instruction::CallFun { .. }
        | Instruction::CallFun2 { .. }
        | Instruction::FuncInfo { .. }
        | Instruction::RecvMarkerReserve { .. }
        | Instruction::RecvMarkerBind { .. }
        | Instruction::RecvMarkerClear { .. }
        | Instruction::RecvMarkerUse { .. } => {}
        // Unconditional in-slice transfers: target only, never fall-through.
        Instruction::Jump { target } => succ.extend(resolve(target)),
        Instruction::Call { label, .. }
        | Instruction::CallOnly { label, .. }
        | Instruction::CallLast { label, .. } => succ.extend(resolve(label)),
        // Park/scan that loop back to the receive head: the `fail` operand is the
        // loop label. No fall-through (Wait parks; LoopRecEnd jumps to the loop).
        Instruction::Wait { fail } | Instruction::LoopRecEnd { fail } => {
            succ.extend(resolve(fail));
        }
        // Multi-way dispatch: the fail label AND every case target. No fall-through.
        Instruction::SelectVal { fail, list, .. } => {
            succ.extend(resolve(fail));
            if let Ok(pairs) = parse_select_pairs(list) {
                for (_, target) in pairs {
                    succ.extend(resolve(target));
                }
            }
        }
        // Conditional edges: the fail/alternate label AND fall-through.
        Instruction::TypeTest { fail, .. }
        | Instruction::Comparison { fail, .. }
        | Instruction::TestArity { fail, .. }
        | Instruction::IsTaggedTuple { fail, .. }
        | Instruction::LoopRec { fail, .. }
        | Instruction::WaitTimeout { fail, .. }
        | Instruction::Try { label: fail, .. }
        | Instruction::Fadd { fail, .. }
        | Instruction::Fsub { fail, .. }
        | Instruction::Fmul { fail, .. }
        | Instruction::Fdiv { fail, .. }
        | Instruction::Fnegate { fail, .. } => {
            succ.extend(resolve(fail));
            succ.extend(fall_through);
        }
        // Bif: fall-through plus a REAL fail label (the `{f,0}` route is a deopt
        // exit, never an edge).
        Instruction::Bif { op, operands } => {
            if let Ok(parsed) = ParsedBif::parse(*op, operands)
                && !is_no_fail_label(parsed.fail)
            {
                succ.extend(resolve(parsed.fail));
            }
            succ.extend(fall_through);
        }
        Instruction::MapOp { op, operands } => {
            let fail = match op {
                MapOp::PutMapAssoc | MapOp::PutMapExact => {
                    parse_put_map_operands(operands).ok().map(|parsed| parsed.0)
                }
                MapOp::GetMapElements => parse_get_map_elements_operands(operands)
                    .ok()
                    .map(|parsed| parsed.0),
                MapOp::HasMapFields => parse_has_map_fields_operands(operands)
                    .ok()
                    .map(|parsed| parsed.0),
            };
            if let Some(fail) = fail {
                succ.extend(resolve(fail));
            }
            succ.extend(fall_through);
        }
        // Straight-line: fall-through only.
        _ => succ.extend(fall_through),
    }
    succ
}

/// Deopt-after-side-effect guard (LEG 1b, CFG-sensitive revision).
///
/// A native deopt restarts the callee interpreted from ITS START, replaying any
/// observable side effect already committed. The hazard is PATH-DEFINED — "an
/// effect EXECUTED before the deopt" — so the honest guard is control-flow
/// reachability, not slice order (the linear scan was only an approximation, and
/// it false-rejected multi-clause functions whose mutually-exclusive clauses each
/// end in a tail call). This is a forward, monotone boolean dataflow over the
/// slice's control-flow graph: `effect_in[i]` is true iff some observable side
/// effect is executed on SOME path reaching `i`. The join is UNION (may-reach):
/// a merge is tainted if ANY incoming edge carries an effect — a must-join would
/// silently admit the diamond hazard (one effect-free arm "washing" the merge).
/// A runtime-deopt-capable instruction is rejected iff an effect can reach it.
/// Rejection is the normal `mark_unsupported` / interpreter fallback — no new
/// channel. Authorities reused unchanged in role: `is_observable_side_effect`
/// (what taints) and `is_runtime_deopt_capable` (what is guarded).
fn reject_deopt_after_side_effect(
    instructions: &[Instruction],
    labels: &HashMap<u32, usize>,
) -> Result<(), JitError> {
    let len = instructions.len();
    let successors: Vec<Vec<usize>> = (0..len)
        .map(|index| control_flow_successors(index, instructions, labels))
        .collect();

    // Forward union dataflow to fixpoint: seed every node, propagate an effect to
    // successors, re-enqueue on any false->true flip (monotone, so it terminates).
    let mut effect_in = vec![false; len];
    let mut worklist: Vec<usize> = (0..len).collect();
    while let Some(index) = worklist.pop() {
        let effect_out = effect_in[index] || is_observable_side_effect(&instructions[index]);
        if effect_out {
            for &successor in &successors[index] {
                if !effect_in[successor] {
                    effect_in[successor] = true;
                    worklist.push(successor);
                }
            }
        }
    }

    for (index, instruction) in instructions.iter().enumerate() {
        if effect_in[index] && is_runtime_deopt_capable(instruction) {
            return Err(JitError::UnsupportedOpcode {
                opcode: format!(
                    "runtime-deopt-capable {} is reachable after an observable side effect \
                     (a deopt would replay the effect on restart)",
                    opcode_name(instruction)
                ),
            });
        }
    }
    Ok(())
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

/// The tail-position wall (JIT-002 R3 tail-only-call-model ruling).
///
/// The tier has NO body-call model: every helper-return call lowering
/// (CallExt/Apply/CallFun/CallFun2) writes X0 then `return_status`
/// unconditionally — it terminates the compiled function and returns the
/// callee's result to the edge — and a plain Call lowers to a bare in-slice
/// jump. A body-position call's continuation is therefore silently dropped. This
/// structural pre-pass check (like label resolution) admits those five opcodes
/// only when immediately followed by `Return`; the `*Only`/`*Last` forms are
/// inherently tail and never reach it. A violation rejects the WHOLE function
/// through the normal `UnsupportedOpcode` path into `mark_unsupported` /
/// interpreter fallback — no new rejection channel.
fn require_tail_position(call: &Instruction, next: Option<&Instruction>) -> Result<(), JitError> {
    if matches!(next, Some(Instruction::Return)) {
        Ok(())
    } else {
        Err(JitError::UnsupportedOpcode {
            opcode: format!(
                "{} in body position (not immediately followed by Return): \
                 the JIT tier has no body-call model",
                opcode_name(call)
            ),
        })
    }
}

pub(crate) fn opcode_name(instruction: &Instruction) -> String {
    match instruction {
        Instruction::Generic { opcode, name, .. } => format!("{name} ({opcode})"),
        other => format!("{other:?}"),
    }
}
