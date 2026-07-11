//! Complete `Instruction` → opcode + operand-stream encoder.
//!
//! The decoder's `instruction_opcode` reverse map covers only the ~20 variants
//! it needs for `opcode_max` validation. This table is total: every one of the
//! 75 `Instruction` variants the decoder can produce round-trips, with the
//! `Generic` catch-all carrying its own opcode for anything decoded as a
//! passthrough. Operand order mirrors `decode::code`'s field assignment exactly.

use crate::loader::decode::{
    BifOp, BinaryOp, ComparisonOp, Instruction, MapOp, Operand, TypeTestOp,
};

use super::compact::{AtomEncoder, encode_operand, push_unsigned};
use super::container::EncodeError;

/// Encodes one instruction (opcode byte + operands) into `out`.
pub(crate) fn encode_instruction(
    out: &mut Vec<u8>,
    instruction: &Instruction,
    atoms: &AtomEncoder<'_>,
) -> Result<(), EncodeError> {
    let opcode = instruction_opcode(instruction)?;
    out.push(opcode);
    encode_operands(out, instruction, atoms)
}

/// The opcode a given instruction encodes to — the encoder's authority for the
/// `Code` header's `opcode_max`.
pub(crate) fn instruction_opcode(instruction: &Instruction) -> Result<u8, EncodeError> {
    let opcode = match instruction {
        Instruction::Label { .. } => 1,
        Instruction::FuncInfo { .. } => 2,
        Instruction::Call { .. } => 4,
        Instruction::CallLast { .. } => 5,
        Instruction::CallOnly { .. } => 6,
        Instruction::CallExt { .. } => 7,
        Instruction::CallExtLast { .. } => 8,
        Instruction::Allocate { .. } => 12,
        Instruction::AllocateHeap { .. } => 13,
        Instruction::AllocateZero { .. } => 14,
        Instruction::TestHeap { .. } => 16,
        Instruction::Deallocate { .. } => 18,
        Instruction::Return => 19,
        Instruction::Send => 20,
        Instruction::RemoveMessage => 21,
        Instruction::Timeout => 22,
        Instruction::LoopRec { .. } => 23,
        Instruction::LoopRecEnd { .. } => 24,
        Instruction::Wait { .. } => 25,
        Instruction::WaitTimeout { .. } => 26,
        Instruction::Comparison { op, .. } => comparison_opcode(*op),
        Instruction::TypeTest { op, .. } => type_test_opcode(*op),
        Instruction::TestArity { .. } => 58,
        Instruction::SelectVal { .. } => 59,
        Instruction::SelectTupleArity { .. } => 60,
        Instruction::Jump { .. } => 61,
        Instruction::Catch { .. } => 62,
        Instruction::CatchEnd { .. } => 63,
        Instruction::Move { .. } => 64,
        Instruction::GetList { .. } => 65,
        Instruction::GetTupleElement { .. } => 66,
        Instruction::PutList { .. } => 69,
        Instruction::Badmatch { .. } => 72,
        Instruction::IfEnd => 73,
        Instruction::CaseEnd { .. } => 74,
        Instruction::CallFun { .. } => 75,
        Instruction::CallExtOnly { .. } => 78,
        Instruction::Fmove { .. } => 96,
        Instruction::Fconv { .. } => 97,
        Instruction::Fadd { .. } => 98,
        Instruction::Fsub { .. } => 99,
        Instruction::Fmul { .. } => 100,
        Instruction::Fdiv { .. } => 101,
        Instruction::Fnegate { .. } => 102,
        Instruction::Try { .. } => 104,
        Instruction::TryEnd { .. } => 105,
        Instruction::TryCase { .. } => 106,
        Instruction::TryCaseEnd { .. } => 107,
        Instruction::Raise { .. } => 108,
        Instruction::Apply { .. } => 112,
        Instruction::ApplyLast { .. } => 113,
        Instruction::Bif { op, .. } => bif_opcode(*op),
        Instruction::BinaryOp { op, .. } => binary_opcode(*op),
        Instruction::MapOp { op, .. } => map_opcode(*op),
        // make_fun2 (103, arity 1) and make_fun3 (171, arity 3) both decode to
        // `MakeFun`; the operand count disambiguates on the way back out.
        Instruction::MakeFun { operands } => match operands.len() {
            1 => 103,
            3 => 171,
            _ => return Err(EncodeError::UnsupportedInstruction),
        },
        Instruction::Trim { .. } => 136,
        Instruction::OnLoad => 149,
        Instruction::Line { .. } => 153,
        Instruction::IsTaggedTuple { .. } => 159,
        Instruction::BuildStacktrace => 160,
        Instruction::RawRaise => 161,
        Instruction::GetHd { .. } => 162,
        Instruction::GetTl { .. } => 163,
        Instruction::PutTuple2 { .. } => 164,
        Instruction::Swap { .. } => 169,
        Instruction::InitYregs { .. } => 172,
        Instruction::RecvMarkerBind { .. } => 173,
        Instruction::RecvMarkerClear { .. } => 174,
        Instruction::RecvMarkerReserve { .. } => 175,
        Instruction::RecvMarkerUse { .. } => 176,
        Instruction::CallFun2 { .. } => 178,
        Instruction::NifStart => 179,
        Instruction::Badrecord { .. } => 180,
        Instruction::UpdateRecord { .. } => 181,
        Instruction::Generic { opcode, .. } => *opcode,
    };
    Ok(opcode)
}

fn encode_operands(
    out: &mut Vec<u8>,
    instruction: &Instruction,
    atoms: &AtomEncoder<'_>,
) -> Result<(), EncodeError> {
    match instruction {
        Instruction::Label { label } => push_unsigned(out, 0, u64::from(*label)),
        Instruction::Return
        | Instruction::Send
        | Instruction::RemoveMessage
        | Instruction::Timeout
        | Instruction::IfEnd
        | Instruction::RawRaise
        | Instruction::OnLoad
        | Instruction::BuildStacktrace
        | Instruction::NifStart => {}
        Instruction::FuncInfo {
            module,
            function,
            arity,
        } => emit(out, atoms, &[module, function, arity])?,
        Instruction::Move {
            source,
            destination,
        } => emit(out, atoms, &[source, destination])?,
        Instruction::Fmove { source, dest } | Instruction::Fconv { source, dest } => {
            emit(out, atoms, &[source, dest])?
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
        } => emit(out, atoms, &[fail, left, right, dest])?,
        Instruction::Fnegate { fail, source, dest } => emit(out, atoms, &[fail, source, dest])?,
        Instruction::Call { arity, label } | Instruction::CallOnly { arity, label } => {
            emit(out, atoms, &[arity, label])?
        }
        Instruction::CallExt { arity, import } | Instruction::CallExtOnly { arity, import } => {
            emit(out, atoms, &[arity, import])?
        }
        Instruction::CallLast {
            arity,
            label,
            deallocate,
        } => emit(out, atoms, &[arity, label, deallocate])?,
        Instruction::CallExtLast {
            arity,
            import,
            deallocate,
        } => emit(out, atoms, &[arity, import, deallocate])?,
        Instruction::Allocate { stack_need, live }
        | Instruction::AllocateZero { stack_need, live } => emit(out, atoms, &[stack_need, live])?,
        Instruction::AllocateHeap {
            stack_need,
            heap_need,
            live,
        } => emit(out, atoms, &[stack_need, heap_need, live])?,
        Instruction::Deallocate { words } => emit(out, atoms, &[words])?,
        Instruction::TestHeap { heap_need, live } => emit(out, atoms, &[heap_need, live])?,
        Instruction::PutList {
            head,
            tail,
            destination,
        } => emit(out, atoms, &[head, tail, destination])?,
        Instruction::PutTuple2 {
            destination,
            elements,
        } => emit(out, atoms, &[destination, elements])?,
        Instruction::GetTupleElement {
            source,
            index,
            destination,
        } => emit(out, atoms, &[source, index, destination])?,
        Instruction::GetList { source, head, tail } => emit(out, atoms, &[source, head, tail])?,
        Instruction::GetHd {
            source,
            destination,
        }
        | Instruction::GetTl {
            source,
            destination,
        } => emit(out, atoms, &[source, destination])?,
        // is_function2 (115) is decoded from three operands into `fail` plus a
        // synthetic `[function, arity]` list; unwrap it back into two operands.
        Instruction::TypeTest {
            op: TypeTestOp::IsFunction2,
            fail,
            value,
        } => {
            let Operand::List(pair) = value else {
                return Err(EncodeError::UnsupportedInstruction);
            };
            if pair.len() != 2 {
                return Err(EncodeError::UnsupportedInstruction);
            }
            emit(out, atoms, &[fail, &pair[0], &pair[1]])?;
        }
        Instruction::TypeTest { fail, value, .. } => emit(out, atoms, &[fail, value])?,
        Instruction::Comparison {
            fail, left, right, ..
        } => emit(out, atoms, &[fail, left, right])?,
        Instruction::TestArity { fail, tuple, arity } => emit(out, atoms, &[fail, tuple, arity])?,
        Instruction::IsTaggedTuple {
            fail,
            value,
            arity,
            tag,
        } => emit(out, atoms, &[fail, value, arity, tag])?,
        Instruction::SelectVal { value, fail, list }
        | Instruction::SelectTupleArity { value, fail, list } => {
            emit(out, atoms, &[value, fail, list])?
        }
        Instruction::Jump { target } => emit(out, atoms, &[target])?,
        Instruction::Bif { operands, .. }
        | Instruction::BinaryOp { operands, .. }
        | Instruction::MapOp { operands, .. }
        | Instruction::MakeFun { operands }
        | Instruction::UpdateRecord { operands }
        | Instruction::Generic { operands, .. } => emit_all(out, atoms, operands)?,
        Instruction::LoopRec { fail, destination } => emit(out, atoms, &[fail, destination])?,
        Instruction::LoopRecEnd { fail } | Instruction::Wait { fail } => emit(out, atoms, &[fail])?,
        Instruction::WaitTimeout { fail, timeout } => emit(out, atoms, &[fail, timeout])?,
        Instruction::RecvMarkerReserve { dest } => emit(out, atoms, &[dest])?,
        Instruction::RecvMarkerBind { marker, reference } => {
            emit(out, atoms, &[marker, reference])?
        }
        Instruction::RecvMarkerClear { marker } | Instruction::RecvMarkerUse { marker } => {
            emit(out, atoms, &[marker])?
        }
        Instruction::Catch { destination, label } | Instruction::Try { destination, label } => {
            emit(out, atoms, &[destination, label])?
        }
        Instruction::CatchEnd { source }
        | Instruction::TryEnd { source }
        | Instruction::TryCase { source }
        | Instruction::TryCaseEnd { source }
        | Instruction::Badmatch { value: source }
        | Instruction::Badrecord { value: source }
        | Instruction::CaseEnd { value: source }
        | Instruction::Line { index: source } => emit(out, atoms, &[source])?,
        Instruction::Raise { stacktrace, reason } => emit(out, atoms, &[stacktrace, reason])?,
        Instruction::Trim { words, remaining } => emit(out, atoms, &[words, remaining])?,
        Instruction::Swap { left, right } => emit(out, atoms, &[left, right])?,
        Instruction::InitYregs { registers } => emit(out, atoms, &[registers])?,
        Instruction::CallFun { arity } | Instruction::Apply { arity } => {
            emit(out, atoms, &[arity])?
        }
        Instruction::CallFun2 {
            function,
            arity,
            destination,
        } => emit(out, atoms, &[function, arity, destination])?,
        Instruction::ApplyLast { arity, deallocate } => emit(out, atoms, &[arity, deallocate])?,
    }
    Ok(())
}

fn emit(
    out: &mut Vec<u8>,
    atoms: &AtomEncoder<'_>,
    operands: &[&Operand],
) -> Result<(), EncodeError> {
    for operand in operands {
        encode_operand(out, operand, atoms)?;
    }
    Ok(())
}

fn emit_all(
    out: &mut Vec<u8>,
    atoms: &AtomEncoder<'_>,
    operands: &[Operand],
) -> Result<(), EncodeError> {
    for operand in operands {
        encode_operand(out, operand, atoms)?;
    }
    Ok(())
}

fn comparison_opcode(op: ComparisonOp) -> u8 {
    match op {
        ComparisonOp::Lt => 39,
        ComparisonOp::Ge => 40,
        ComparisonOp::Eq => 41,
        ComparisonOp::Ne => 42,
        ComparisonOp::EqExact => 43,
        ComparisonOp::NeExact => 44,
    }
}

fn type_test_opcode(op: TypeTestOp) -> u8 {
    match op {
        TypeTestOp::IsInteger => 45,
        TypeTestOp::IsFloat => 46,
        TypeTestOp::IsNumber => 47,
        TypeTestOp::IsAtom => 48,
        TypeTestOp::IsPid => 49,
        TypeTestOp::IsReference => 50,
        TypeTestOp::IsPort => 51,
        TypeTestOp::IsNil => 52,
        TypeTestOp::IsBinary => 53,
        TypeTestOp::IsList => 55,
        TypeTestOp::IsNonemptyList => 56,
        TypeTestOp::IsTuple => 57,
        TypeTestOp::IsFunction => 77,
        TypeTestOp::IsBoolean => 114,
        TypeTestOp::IsFunction2 => 115,
        TypeTestOp::IsBitstr => 129,
        TypeTestOp::IsMap => 156,
    }
}

fn bif_opcode(op: BifOp) -> u8 {
    match op {
        BifOp::Bif0 => 9,
        BifOp::Bif1 => 10,
        BifOp::Bif2 => 11,
        BifOp::GcBif1 => 124,
        BifOp::GcBif2 => 125,
        BifOp::GcBif3 => 152,
    }
}

fn binary_opcode(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::BsGetInteger2 => 117,
        BinaryOp::BsGetFloat2 => 118,
        BinaryOp::BsGetBinary2 => 119,
        BinaryOp::BsSkipBits2 => 120,
        BinaryOp::BsTestTail2 => 121,
        BinaryOp::BsTestUnit => 131,
        BinaryOp::BsMatchString => 132,
        BinaryOp::BsInitWritable => 133,
        BinaryOp::BsGetUtf8 => 138,
        BinaryOp::BsSkipUtf8 => 139,
        BinaryOp::BsGetUtf16 => 140,
        BinaryOp::BsSkipUtf16 => 141,
        BinaryOp::BsGetUtf32 => 142,
        BinaryOp::BsSkipUtf32 => 143,
        BinaryOp::BsGetTail => 165,
        BinaryOp::BsStartMatch3 => 166,
        BinaryOp::BsGetPosition => 167,
        BinaryOp::BsSetPosition => 168,
        BinaryOp::BsStartMatch4 => 170,
        BinaryOp::BsCreateBin => 177,
        BinaryOp::BsMatch => 182,
    }
}

fn map_opcode(op: MapOp) -> u8 {
    match op {
        MapOp::PutMapAssoc => 154,
        MapOp::PutMapExact => 155,
        MapOp::HasMapFields => 157,
        MapOp::GetMapElements => 158,
    }
}
