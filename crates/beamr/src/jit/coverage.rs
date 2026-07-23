//! Instruction-classification tier for the demand-JIT compiler.
//!
//! The four classification tables of record — [`coverage`],
//! [`is_observable_side_effect`], [`is_runtime_deopt_capable`], and
//! [`is_no_fail_label`] — live together HERE, deliberately beside each other:
//! each is EXHAUSTIVE WITH NO WILDCARD ARM, so a later wave adding an
//! `Instruction` variant must classify it in every table in this one module or
//! compilation breaks. The translation-planning machinery that consumes these
//! tables lives in `ir_control`.

use crate::loader::Instruction;

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
        | Instruction::CallFun2 { .. }
        // -- Supported: LEG 1c A2 — func_info retained as the function_clause
        // -- landing pad. Lowered as a DEOPT terminal (RecvMarker precedent): the
        // -- restarted interpreter raises error:function_clause. Reached only via
        // -- a dispatch fail edge; normal calls enter at the label AFTER it.
        | Instruction::FuncInfo { .. } => Coverage::Supported,

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
/// cannot rot silently. The observable set is the effects a deopt-restart would
/// wrongly REPEAT: a message send (duplication), an ACCEPTED message
/// (`RemoveMessage` — the message-loss consume), a mutated receive marker, a
/// consumed receive timeout, and every call form (arbitrary callee effects).
///
/// The receive PEEK/PARK edges (`LoopRec`/`LoopRecEnd`/`Wait`/`WaitTimeout`) are
/// NOT observable: peeking/scanning/parking is idempotent on replay — the message
/// is not consumed until `RemoveMessage`, which happens only on the matched path
/// (which exits the receive loop). Treating the peek as pure is what admits a
/// single blocking receive while still rejecting the true loss path (a prior
/// `RemoveMessage` reaching a later receive's peek/wait deopt edge).
/// Register/heap/stack mutation is likewise NOT observable — a restart redoes it
/// harmlessly.
pub(crate) fn is_observable_side_effect(instruction: &Instruction) -> bool {
    match instruction {
        Instruction::Send
        | Instruction::RemoveMessage
        | Instruction::Timeout
        | Instruction::RecvMarkerReserve { .. }
        | Instruction::RecvMarkerBind { .. }
        | Instruction::RecvMarkerClear { .. }
        | Instruction::RecvMarkerUse { .. }
        | Instruction::CallExt { .. }
        | Instruction::CallExtOnly { .. }
        | Instruction::CallExtLast { .. }
        | Instruction::Apply { .. }
        | Instruction::ApplyLast { .. }
        | Instruction::CallFun { .. }
        | Instruction::CallFun2 { .. } => true,
        // Local Call/CallOnly/CallLast lower to an IN-SLICE jump to a validated
        // in-slice label; their target's real effects are tracked by the dataflow's
        // control-flow edge into it, so the transfer itself is not an opaque
        // effect. (Marking them effects wrongly taints a self-tail-recursion loop
        // head through the CallLast back edge.) External CallExt*/Apply*/CallFun*
        // reach opaque callees and stay observable above.
        Instruction::Call { .. }
        | Instruction::CallOnly { .. }
        | Instruction::CallLast { .. }
        | Instruction::LoopRec { .. }
        | Instruction::LoopRecEnd { .. }
        | Instruction::Wait { .. }
        | Instruction::WaitTimeout { .. }
        | Instruction::Label { .. }
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
        | Instruction::Fmove { .. }
        // LEG 1c A2: func_info is lowered as an unconditional DEOPT terminal.
        | Instruction::FuncInfo { .. } => true,
        Instruction::Label { .. }
        | Instruction::Line { .. }
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
