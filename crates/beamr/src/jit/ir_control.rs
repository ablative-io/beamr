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
pub enum Coverage {
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
///
/// The classification is PER-OPCODE: `Supported` means the opcode is lowerable
/// under the tier's tail-only call model. Body-position structure (a call must
/// be in tail position — see `require_tail_position`) is the pre-pass wall's
/// business, like label resolution, and does not move an opcode's class here.
pub fn coverage(instruction: &Instruction) -> Coverage {
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

/// Whether re-executing `instruction` would repeat an OBSERVABLE side effect.
///
/// Table-adjacent authority for the no-fail-Bif purity guard (JIT-002 R3 BIF
/// NO-FAIL RULING): an `{f,0}` arithmetic Bif routes to deopt, and deopt restarts
/// the callee interpreted from its start, so re-execution of any side-effecting
/// instruction that already ran would double the effect. The classification is
/// EXHAUSTIVE WITH NO WILDCARD, sitting beside `coverage` so a later wave adding
/// an `Instruction` variant must decide its side-effect class here — the guard
/// cannot rot silently. The observable set is message send + the mailbox/receive
/// family (a sent message, a consumed/advanced mailbox, a mutated receive
/// marker) plus every call form (arbitrary callee effects; calls are tail-only
/// under R3's wall so they cannot actually precede a Bif, classified
/// conservatively). Register/heap/stack mutation is NOT observable — a restart
/// redoes it harmlessly.
pub(crate) fn is_observable_side_effect(instruction: &Instruction) -> bool {
    match instruction {
        Instruction::Send
        | Instruction::RemoveMessage
        | Instruction::Timeout
        | Instruction::LoopRec { .. }
        | Instruction::LoopRecEnd { .. }
        | Instruction::Wait { .. }
        | Instruction::WaitTimeout { .. }
        | Instruction::RecvMarkerReserve { .. }
        | Instruction::RecvMarkerBind { .. }
        | Instruction::RecvMarkerClear { .. }
        | Instruction::RecvMarkerUse { .. }
        | Instruction::Call { .. }
        | Instruction::CallOnly { .. }
        | Instruction::CallLast { .. }
        | Instruction::CallExt { .. }
        | Instruction::CallExtOnly { .. }
        | Instruction::CallExtLast { .. }
        | Instruction::Apply { .. }
        | Instruction::ApplyLast { .. }
        | Instruction::CallFun { .. }
        | Instruction::CallFun2 { .. } => true,
        Instruction::Label { .. }
        | Instruction::FuncInfo { .. }
        | Instruction::Move { .. }
        | Instruction::Swap { .. }
        | Instruction::Bif { .. }
        | Instruction::TypeTest { .. }
        | Instruction::Comparison { .. }
        | Instruction::TestArity { .. }
        | Instruction::IsTaggedTuple { .. }
        | Instruction::SelectVal { .. }
        | Instruction::SelectTupleArity { .. }
        | Instruction::Jump { .. }
        | Instruction::Try { .. }
        | Instruction::TryEnd { .. }
        | Instruction::TryCase { .. }
        | Instruction::TryCaseEnd { .. }
        | Instruction::Catch { .. }
        | Instruction::CatchEnd { .. }
        | Instruction::Return
        | Instruction::Fmove { .. }
        | Instruction::Fconv { .. }
        | Instruction::Fadd { .. }
        | Instruction::Fsub { .. }
        | Instruction::Fmul { .. }
        | Instruction::Fdiv { .. }
        | Instruction::Fnegate { .. }
        | Instruction::PutList { .. }
        | Instruction::PutTuple2 { .. }
        | Instruction::GetTupleElement { .. }
        | Instruction::GetList { .. }
        | Instruction::GetHd { .. }
        | Instruction::GetTl { .. }
        | Instruction::MakeFun { .. }
        | Instruction::BinaryOp { .. }
        | Instruction::MapOp { .. }
        | Instruction::Allocate { .. }
        | Instruction::AllocateHeap { .. }
        | Instruction::AllocateZero { .. }
        | Instruction::Deallocate { .. }
        | Instruction::TestHeap { .. }
        | Instruction::InitYregs { .. }
        | Instruction::Trim { .. }
        | Instruction::Line { .. }
        | Instruction::Badmatch { .. }
        | Instruction::Badrecord { .. }
        | Instruction::CaseEnd { .. }
        | Instruction::IfEnd
        | Instruction::Raise { .. }
        | Instruction::RawRaise
        | Instruction::BuildStacktrace
        | Instruction::OnLoad
        | Instruction::NifStart
        | Instruction::UpdateRecord { .. }
        | Instruction::Generic { .. } => false,
    }
}

/// Whether lowering `instruction` CAN emit a runtime deopt branch.
///
/// Table-adjacent authority for the deopt-after-side-effect pre-pass guard
/// (ADMISSION ARC LEG 1b, generalizing the JIT-002 R3 BIF NO-FAIL RULING from
/// "`{f,0}` arithmetic Bif" to "any runtime-deopt-capable instruction"). A native
/// deopt restarts the callee interpreted from its start (interpreter/opcodes/
/// core.rs:850 -> Ok(None) -> re-enter at the entry label), so a deopt reached
/// AFTER an observable side effect replays that effect. `TranslationPlan::new`
/// rejects any slice where an instruction classified `true` here follows an
/// [`is_observable_side_effect`] instruction.
///
/// EXHAUSTIVE WITH NO WILDCARD, sitting beside [`coverage`] and
/// [`is_observable_side_effect`]: a later wave adding an `Instruction` variant
/// MUST classify its deopt-capability here or compilation breaks — the guard
/// cannot rot silently. Classification is derived from the lowerings, not from
/// memory; `true` is the conservative (sound) default when a lowering's deopt
/// edge is in doubt.
///
/// The `true` set, with its deopt edge, is: arithmetic `Bif` (the typed-overflow
/// branch `ir_typed.rs:32-55` and the `{f,0}` no-fail route both target the deopt
/// block); the frame ops `Allocate`/`AllocateHeap`/`AllocateZero`/`Deallocate`/
/// `TestHeap`/`Trim` (`frame_guard` deopts on a frame-helper failure); the
/// receive peek/wait edges `LoopRec`/`Wait`/`WaitTimeout` (deopt on an unexpected
/// mailbox status, `ir_message.rs:64,197`); the recv-marker ops
/// `RecvMarkerReserve`/`RecvMarkerBind`/`RecvMarkerClear`/`RecvMarkerUse` (an
/// UNCONDITIONAL deopt, `dispatch_core.rs:411-417`); every helper-return /
/// frame-teardown call form `CallExt`/`CallExtOnly`/`CallExtLast`/`CallLast`/
/// `CallFun`/`CallFun2`/`Apply`/`ApplyLast` (`handle_helper_return` / the tail
/// `dealloc` `frame_guard` deopt); the heap-allocating data ops `PutList`/
/// `PutTuple2`/`MakeFun` and `BinaryOp` (deopt on a null heap allocation); and
/// `Fmove` (deopt on a zero/invalid float box, `ir_float.rs:96`).
///
/// Everything else is `false`: pure register/heap reads, the guards and the plain
/// float/map ops (which branch only to REAL in-slice fail labels, never deopt),
/// `Send`/`RemoveMessage`/`Timeout`/`LoopRecEnd` (observable but non-deopting
/// lowerings), the plain in-slice `Call`/`CallOnly` jumps, the exception-seam
/// ops, and every non-`Supported` variant (never lowered — the pre-pass rejects
/// the whole function first, so their class never gates a live decision).
pub(crate) fn is_runtime_deopt_capable(instruction: &Instruction) -> bool {
    match instruction {
        Instruction::Bif { .. }
        | Instruction::Allocate { .. }
        | Instruction::AllocateHeap { .. }
        | Instruction::AllocateZero { .. }
        | Instruction::Deallocate { .. }
        | Instruction::TestHeap { .. }
        | Instruction::Trim { .. }
        | Instruction::LoopRec { .. }
        | Instruction::Wait { .. }
        | Instruction::WaitTimeout { .. }
        | Instruction::RecvMarkerReserve { .. }
        | Instruction::RecvMarkerBind { .. }
        | Instruction::RecvMarkerClear { .. }
        | Instruction::RecvMarkerUse { .. }
        | Instruction::CallExt { .. }
        | Instruction::CallExtOnly { .. }
        | Instruction::CallExtLast { .. }
        | Instruction::CallLast { .. }
        | Instruction::CallFun { .. }
        | Instruction::CallFun2 { .. }
        | Instruction::Apply { .. }
        | Instruction::ApplyLast { .. }
        | Instruction::PutList { .. }
        | Instruction::PutTuple2 { .. }
        | Instruction::MakeFun { .. }
        | Instruction::BinaryOp { .. }
        | Instruction::Fmove { .. } => true,
        Instruction::Label { .. }
        | Instruction::Line { .. }
        | Instruction::FuncInfo { .. }
        | Instruction::Move { .. }
        | Instruction::Swap { .. }
        | Instruction::Jump { .. }
        | Instruction::Return
        | Instruction::Call { .. }
        | Instruction::CallOnly { .. }
        | Instruction::TypeTest { .. }
        | Instruction::Comparison { .. }
        | Instruction::TestArity { .. }
        | Instruction::IsTaggedTuple { .. }
        | Instruction::SelectVal { .. }
        | Instruction::GetList { .. }
        | Instruction::GetHd { .. }
        | Instruction::GetTl { .. }
        | Instruction::GetTupleElement { .. }
        | Instruction::InitYregs { .. }
        | Instruction::Send
        | Instruction::RemoveMessage
        | Instruction::Timeout
        | Instruction::LoopRecEnd { .. }
        | Instruction::MapOp { .. }
        | Instruction::Fconv { .. }
        | Instruction::Fadd { .. }
        | Instruction::Fsub { .. }
        | Instruction::Fmul { .. }
        | Instruction::Fdiv { .. }
        | Instruction::Fnegate { .. }
        | Instruction::Try { .. }
        | Instruction::TryEnd { .. }
        | Instruction::TryCase { .. }
        // -- non-Supported variants: never lowered (pre-pass rejects first) --
        | Instruction::SelectTupleArity { .. }
        | Instruction::Catch { .. }
        | Instruction::CatchEnd { .. }
        | Instruction::TryCaseEnd { .. }
        | Instruction::Raise { .. }
        | Instruction::RawRaise
        | Instruction::BuildStacktrace
        | Instruction::Badmatch { .. }
        | Instruction::Badrecord { .. }
        | Instruction::CaseEnd { .. }
        | Instruction::IfEnd
        | Instruction::UpdateRecord { .. }
        | Instruction::OnLoad
        | Instruction::NifStart
        | Instruction::Generic { .. } => false,
    }
}

/// erlc's `{f,0}` fail-label sentinel: "no local handler" — emitted on all
/// body-position arithmetic. It is NOT a real label; it routes to the deopt
/// block in the lowering.
pub(crate) fn is_no_fail_label(operand: &crate::loader::decode::Operand) -> bool {
    matches!(operand, crate::loader::decode::Operand::Label(0))
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
        // Deopt-after-side-effect guard (LEG 1b): a native deopt restarts the
        // callee interpreted from its start, replaying any side effect already
        // committed. Reject the WHOLE function (normal `mark_unsupported` /
        // interpreter fallback — no new channel) when a runtime-deopt-capable
        // instruction follows an observable side effect in the slice. The two
        // authorities are the table-adjacent `is_runtime_deopt_capable` and
        // `is_observable_side_effect`; this walks them in order so the check is
        // O(n) and no earlier side effect can be missed.
        let mut side_effect_seen = false;
        for (index, instruction) in instructions.iter().enumerate() {
            if side_effect_seen && is_runtime_deopt_capable(instruction) {
                return Err(JitError::UnsupportedOpcode {
                    opcode: format!(
                        "runtime-deopt-capable {} follows an observable side effect \
                         (deopt-restart would replay the effect)",
                        opcode_name(instruction)
                    ),
                });
            }
            side_effect_seen |= is_observable_side_effect(instruction);
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
