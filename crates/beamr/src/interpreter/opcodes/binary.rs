//! Binary construction and matching opcode handlers.

use crate::error::ExecError;
use crate::interpreter::InstructionOutcome;
use crate::loader::decode::compact::Operand;
use crate::loader::decode::{BinaryOp, Literal};
use crate::module::Module;
use crate::process::{CodePosition, Process};
use crate::term::Term;
use crate::term::binary::{Binary, packed_word_count, write_binary};
use crate::term::boxed::BoxedTag;

use super::binary_state::{
    BUILDER_META_WORDS, BinaryBuilder, MATCH_CONTEXT_WORDS, MatchContext, boxed_header, heap_slice,
};
use super::core;

/// Dispatch a binary construction or matching opcode.
pub fn binary_op(
    process: &mut Process,
    module: &Module,
    op: BinaryOp,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    match op {
        BinaryOp::BsInitWritable | BinaryOp::BsCreateBin => bs_init_or_create(process, operands),
        BinaryOp::BsStartMatch3 | BinaryOp::BsStartMatch4 => {
            bs_start_match(process, module, operands)
        }
        BinaryOp::BsGetInteger2 => bs_get_integer(process, module, operands),
        BinaryOp::BsGetBinary2 => bs_get_binary(process, module, operands),
        BinaryOp::BsMatchString => bs_match_string(process, module, operands),
        BinaryOp::BsTestTail2 => bs_test_tail(process, module, operands),
        other => Err(ExecError::UnsupportedOpcode {
            name: binary_opcode_name(other),
        }),
    }
}

fn bs_init_or_create(
    process: &mut Process,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    match operands {
        [size, destination] => {
            let capacity = core::operand_usize(size, "binary builder size")?;
            let term = allocate_builder(process, capacity)?;
            core::write_term(process, destination, term)?;
            Ok(InstructionOutcome::Continue)
        }
        [destination, size, segments @ ..] => {
            let capacity = core::operand_usize(size, "binary builder size")?;
            let builder = allocate_builder(process, capacity)?;
            for segment in segments {
                append_create_bin_segment(process, builder, segment)?;
            }
            let binary = finalize_builder(process, builder)?;
            core::write_term(process, destination, binary)?;
            Ok(InstructionOutcome::Continue)
        }
        _ => Err(ExecError::InvalidOperand("bs_init2 operands")),
    }
}

fn bs_start_match(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, source, destination) = match operands {
        [fail, source, destination] => (fail, source, destination),
        [fail, source, _live, destination] => (fail, source, destination),
        _ => return Err(ExecError::InvalidOperand("bs_start_match operands")),
    };
    let source = core::read_term(process, source)?;
    let Some(binary) = Binary::new(source) else {
        return jump_label(module, fail);
    };

    let ptr = process
        .heap_mut()
        .alloc(MATCH_CONTEXT_WORDS)
        .map_err(ExecError::from)?;
    let heap = heap_slice(ptr, MATCH_CONTEXT_WORDS);
    heap[0] = boxed_header(BoxedTag::MatchContext, MATCH_CONTEXT_WORDS - 1);
    heap[1] = 0;
    heap[2] = (binary.len() * u8::BITS as usize) as u64;
    heap[3] = source.raw();
    core::write_term(process, destination, Term::boxed_ptr(heap.as_ptr()))?;
    Ok(InstructionOutcome::Continue)
}

fn bs_get_integer(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, size, unit, flags, destination) =
        parse_get_operands(operands, "bs_get_integer2")?;
    let size_bits = segment_bits(size, unit)?;
    let endian = Endian::from_flags(flags);
    let context_term = core::read_term(process, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    if !size_bits.is_multiple_of(u8::BITS as usize)
        || !context.position_bits().is_multiple_of(u8::BITS as usize)
    {
        return Err(ExecError::Badarg);
    }
    if !context.has_bits(size_bits) {
        return jump_label(module, fail);
    }

    let bytes = context.slice(size_bits).ok_or(ExecError::Badarg)?;
    let value = decode_integer(bytes, endian)?;
    let term = Term::try_small_int(value).ok_or(ExecError::Badarg)?;
    core::write_term(process, destination, term)?;
    context.set_position_bits(context.position_bits() + size_bits);
    Ok(InstructionOutcome::Continue)
}

fn bs_get_binary(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, size, unit, _flags, destination) =
        parse_get_operands(operands, "bs_get_binary2")?;
    let size_bits = segment_bits(size, unit)?;
    let context_term = core::read_term(process, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    if !size_bits.is_multiple_of(u8::BITS as usize)
        || !context.position_bits().is_multiple_of(u8::BITS as usize)
    {
        return Err(ExecError::Badarg);
    }
    if !context.has_bits(size_bits) {
        return jump_label(module, fail);
    }

    let bytes = context.slice(size_bits).ok_or(ExecError::Badarg)?;
    let words = 2 + packed_word_count(bytes.len());
    if process.heap().available() < words {
        return Err(ExecError::GcNeeded {
            requested: words,
            available: process.heap().available(),
        });
    }
    let ptr = process.heap_mut().alloc(words).map_err(ExecError::from)?;
    let heap = heap_slice(ptr, words);
    let binary = write_binary(heap, bytes).ok_or(ExecError::Badarg)?;
    core::write_term(process, destination, binary)?;
    context.set_position_bits(context.position_bits() + size_bits);
    Ok(InstructionOutcome::Continue)
}

fn bs_match_string(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, bit_len, literal) = match operands {
        [fail, context, bit_len, literal] => (fail, context, bit_len, literal),
        _ => return Err(ExecError::InvalidOperand("bs_match_string operands")),
    };
    let bit_len = core::operand_usize(bit_len, "bs_match_string bit length")?;
    if !bit_len.is_multiple_of(u8::BITS as usize) {
        return Err(ExecError::Badarg);
    }
    let expected = literal_bytes(module, literal, bit_len / u8::BITS as usize)?;
    let context_term = core::read_term(process, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    if !context.position_bits().is_multiple_of(u8::BITS as usize) || !context.has_bits(bit_len) {
        return jump_label(module, fail);
    }
    let candidate = context.slice(bit_len).ok_or(ExecError::Badarg)?;
    if candidate != expected {
        return jump_label(module, fail);
    }
    context.set_position_bits(context.position_bits() + bit_len);
    Ok(InstructionOutcome::Continue)
}

fn bs_test_tail(
    process: &Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, expected) = match operands {
        [fail, context, expected] => (fail, context, expected),
        _ => return Err(ExecError::InvalidOperand("bs_test_tail2 operands")),
    };
    let expected = core::operand_usize(expected, "bs_test_tail2 remaining bits")?;
    let context_term = core::read_term(process, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    if context.remaining_bits() == expected {
        Ok(InstructionOutcome::Continue)
    } else {
        jump_label(module, fail)
    }
}

fn append_create_bin_segment(
    process: &mut Process,
    builder: Term,
    segment: &Operand,
) -> Result<(), ExecError> {
    let Operand::List(fields) = segment else {
        return Err(ExecError::InvalidOperand("bs_create_bin segment"));
    };
    match fields.as_slice() {
        [Operand::Atom(None), value, size, unit, flags] => {
            bs_put_integer(process, builder, value, size, unit, flags)
        }
        [Operand::Atom(None), source] => bs_put_binary(process, builder, source),
        _ => Err(ExecError::InvalidOperand("bs_create_bin segment")),
    }
}

pub(crate) fn bs_put_integer(
    process: &mut Process,
    builder: Term,
    value: &Operand,
    size: &Operand,
    unit: &Operand,
    flags: &Operand,
) -> Result<(), ExecError> {
    let value = core::read_term(process, value)?;
    let value = value.as_small_int().ok_or(ExecError::Badarg)?;
    let size_bits = segment_bits(size, unit)?;
    let endian = Endian::from_flags(flags);
    if size_bits == 0 || !size_bits.is_multiple_of(u8::BITS as usize) {
        return Err(ExecError::Badarg);
    }
    let byte_count = size_bits / u8::BITS as usize;
    let builder = BinaryBuilder::new(builder).ok_or(ExecError::Badarg)?;
    let start = builder.write_position_bits();
    if !start.is_multiple_of(u8::BITS as usize) || !builder.can_append(size_bits) {
        return Err(ExecError::Badarg);
    }
    let bytes = encode_integer(value, byte_count, endian)?;
    builder.write_bytes(start / u8::BITS as usize, &bytes);
    builder.set_write_position_bits(start + size_bits);
    Ok(())
}

pub(crate) fn bs_put_binary(
    process: &mut Process,
    builder: Term,
    source: &Operand,
) -> Result<(), ExecError> {
    let source = core::read_term(process, source)?;
    let binary = Binary::new(source).ok_or(ExecError::Badarg)?;
    let bytes = binary.as_bytes();
    let size_bits = bytes.len() * u8::BITS as usize;
    let builder = BinaryBuilder::new(builder).ok_or(ExecError::Badarg)?;
    let start = builder.write_position_bits();
    if !start.is_multiple_of(u8::BITS as usize) || !builder.can_append(size_bits) {
        return Err(ExecError::Badarg);
    }
    builder.write_bytes(start / u8::BITS as usize, bytes);
    builder.set_write_position_bits(start + size_bits);
    Ok(())
}

pub(crate) fn finalize_builder(process: &mut Process, builder: Term) -> Result<Term, ExecError> {
    let builder = BinaryBuilder::new(builder).ok_or(ExecError::Badarg)?;
    if !builder
        .write_position_bits()
        .is_multiple_of(u8::BITS as usize)
    {
        return Err(ExecError::Badarg);
    }
    let byte_len = builder.write_position_bits() / u8::BITS as usize;
    let bytes = builder.bytes(byte_len).ok_or(ExecError::Badarg)?;
    let words = 2 + packed_word_count(byte_len);
    let ptr = process.heap_mut().alloc(words).map_err(ExecError::from)?;
    let heap = heap_slice(ptr, words);
    write_binary(heap, bytes).ok_or(ExecError::Badarg)
}

fn allocate_builder(process: &mut Process, capacity: usize) -> Result<Term, ExecError> {
    let words = BUILDER_META_WORDS
        .checked_add(packed_word_count(capacity))
        .ok_or(ExecError::InvalidOperand("binary builder size"))?;
    if process.heap().available() < words {
        return Err(ExecError::GcNeeded {
            requested: words,
            available: process.heap().available(),
        });
    }
    let ptr = process.heap_mut().alloc(words).map_err(ExecError::from)?;
    let heap = heap_slice(ptr, words);
    heap[0] = boxed_header(BoxedTag::BinaryBuilder, words - 1);
    heap[1] = 0;
    heap[2] = capacity as u64;
    Ok(Term::boxed_ptr(heap.as_ptr()))
}

#[derive(Copy, Clone)]
enum Endian {
    Big,
    Little,
}

impl Endian {
    fn from_flags(flags: &Operand) -> Self {
        match flags {
            Operand::Unsigned(1) | Operand::Integer(1) => Self::Little,
            Operand::List(items) if items.iter().any(is_little_flag) => Self::Little,
            _ => Self::Big,
        }
    }
}

fn parse_get_operands<'a>(
    operands: &'a [Operand],
    context: &'static str,
) -> Result<
    (
        &'a Operand,
        &'a Operand,
        &'a Operand,
        &'a Operand,
        &'a Operand,
        &'a Operand,
    ),
    ExecError,
> {
    match operands {
        [fail, match_context, _live, size, unit, flags, destination] => {
            Ok((fail, match_context, size, unit, flags, destination))
        }
        [fail, match_context, size, unit, flags, destination] => {
            Ok((fail, match_context, size, unit, flags, destination))
        }
        _ => Err(ExecError::InvalidOperand(context)),
    }
}

fn is_little_flag(flag: &Operand) -> bool {
    matches!(flag, Operand::Unsigned(1) | Operand::Integer(1))
}

fn segment_bits(size: &Operand, unit: &Operand) -> Result<usize, ExecError> {
    let size = core::operand_usize(size, "segment size")?;
    let unit = core::operand_usize(unit, "segment unit")?;
    size.checked_mul(unit)
        .ok_or(ExecError::InvalidOperand("segment size"))
}

fn encode_integer(value: i64, byte_count: usize, endian: Endian) -> Result<Vec<u8>, ExecError> {
    if byte_count > std::mem::size_of::<i64>() {
        return Err(ExecError::Badarg);
    }
    let bits = byte_count * u8::BITS as usize;
    if bits < i64::BITS as usize && (value < 0 || (value as u64) >= (1_u64 << bits)) {
        return Err(ExecError::Badarg);
    }
    let bytes = match endian {
        Endian::Big => value.to_be_bytes()[std::mem::size_of::<i64>() - byte_count..].to_vec(),
        Endian::Little => value.to_le_bytes()[..byte_count].to_vec(),
    };
    Ok(bytes)
}

fn decode_integer(bytes: &[u8], endian: Endian) -> Result<i64, ExecError> {
    if bytes.len() > std::mem::size_of::<i64>() {
        return Err(ExecError::Badarg);
    }
    let mut full = [0_u8; 8];
    match endian {
        Endian::Big => full[8 - bytes.len()..].copy_from_slice(bytes),
        Endian::Little => full[..bytes.len()].copy_from_slice(bytes),
    }
    Ok(match endian {
        Endian::Big => u64::from_be_bytes(full) as i64,
        Endian::Little => u64::from_le_bytes(full) as i64,
    })
}

fn literal_bytes<'a>(
    module: &'a Module,
    operand: &'a Operand,
    byte_len: usize,
) -> Result<&'a [u8], ExecError> {
    match operand {
        Operand::Literal(Literal::Binary(bytes) | Literal::String(bytes)) => bytes
            .get(..byte_len)
            .filter(|bytes| bytes.len() == byte_len)
            .ok_or(ExecError::Badarg),
        offset => {
            let offset = core::operand_usize(offset, "string table offset")?;
            let end = offset
                .checked_add(byte_len)
                .ok_or(ExecError::InvalidOperand("string table offset"))?;
            module
                .string_table
                .get(offset..end)
                .ok_or(ExecError::Badarg)
        }
    }
}

fn jump_label(module: &Module, label: &Operand) -> Result<InstructionOutcome, ExecError> {
    let label = core::operand_label(label)?;
    Ok(InstructionOutcome::Jump(CodePosition {
        module: module.name,
        instruction_pointer: core::label_ip(module, label)?,
    }))
}

fn binary_opcode_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::BsGetFloat2 => "bs_get_float2",
        BinaryOp::BsSkipBits2 => "bs_skip_bits2",
        BinaryOp::BsTestUnit => "bs_test_unit",
        BinaryOp::BsGetUtf8 => "bs_get_utf8",
        BinaryOp::BsSkipUtf8 => "bs_skip_utf8",
        BinaryOp::BsGetUtf16 => "bs_get_utf16",
        BinaryOp::BsSkipUtf16 => "bs_skip_utf16",
        BinaryOp::BsGetUtf32 => "bs_get_utf32",
        BinaryOp::BsSkipUtf32 => "bs_skip_utf32",
        BinaryOp::BsGetTail => "bs_get_tail",
        BinaryOp::BsGetPosition => "bs_get_position",
        BinaryOp::BsSetPosition => "bs_set_position",
        BinaryOp::BsMatch => "bs_match",
        _ => "binary_op",
    }
}

#[cfg(test)]
mod tests;
