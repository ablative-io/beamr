use crate::error::ExecError;
use crate::interpreter::InstructionOutcome;
use crate::loader::decode::Literal;
use crate::loader::decode::compact::Operand;
use crate::module::Module;
use crate::process::Process;
use crate::term::Term;
use crate::term::binary::{Binary, packed_word_count, write_binary};
use crate::term::boxed::{BoxedHeader, BoxedTag, write_float};

use super::core;
use super::{MATCH_CONTEXT_WORDS, boxed_tag, heap_slice, jump_label, read_word, write_word};

pub(super) fn bs_start_match(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, source, destination) = match operands {
        [fail, source, destination] => (fail, source, destination),
        [fail, source, _live, destination] => (fail, source, destination),
        _ => return Err(ExecError::InvalidOperand("bs_start_match operands")),
    };
    let source = core::read_term(process, module, source)?;
    let Some(binary) = Binary::new(source) else {
        return jump_label(module, fail);
    };

    let ptr = process
        .heap_mut()
        .alloc(MATCH_CONTEXT_WORDS)
        .map_err(ExecError::from)?;
    let heap = heap_slice(ptr, MATCH_CONTEXT_WORDS);
    heap[0] = BoxedHeader::new(BoxedTag::MatchContext, MATCH_CONTEXT_WORDS - 1);
    heap[1] = 0;
    heap[2] = (binary.len() * u8::BITS as usize) as u64;
    heap[3] = source.raw();
    core::write_term(process, destination, Term::boxed_ptr(heap.as_ptr()))?;
    Ok(InstructionOutcome::Continue)
}

pub(super) fn bs_get_integer(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, size, unit, flags, destination) =
        parse_get_operands(operands, "bs_get_integer2")?;
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let Some(term) = get_integer_term(context, size, unit, flags)? else {
        return jump_label(module, fail);
    };
    core::write_term(process, destination, term)?;
    context.advance(segment_bits(size, unit)?)?;
    Ok(InstructionOutcome::Continue)
}

pub(super) fn bs_get_binary(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, size, unit, _flags, destination) =
        parse_get_operands(operands, "bs_get_binary2")?;
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let size_bits = segment_bits(size, unit)?;
    let Some(binary) = copy_binary_segment(process, context, size_bits)? else {
        return jump_label(module, fail);
    };
    core::write_term(process, destination, binary)?;
    context.advance(size_bits)?;
    Ok(InstructionOutcome::Continue)
}

pub(super) fn bs_match_string(
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
    let context_term = core::read_term(process, module, context)?;
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

pub(super) fn bs_test_tail(
    process: &Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, expected) = match operands {
        [fail, context, expected] => (fail, context, expected),
        _ => return Err(ExecError::InvalidOperand("bs_test_tail2 operands")),
    };
    let expected = core::operand_usize(expected, "bs_test_tail2 remaining bits")?;
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    if context.remaining_bits() == expected {
        Ok(InstructionOutcome::Continue)
    } else {
        jump_label(module, fail)
    }
}

pub(super) fn bs_skip_bits(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, size, unit, _flags) = match operands {
        [fail, context, size, unit, flags] => (fail, context, size, unit, flags),
        _ => return Err(ExecError::InvalidOperand("bs_skip_bits2 operands")),
    };
    let bits = segment_bits(size, unit)?;
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    if !context.has_bits(bits) {
        return jump_label(module, fail);
    }
    context.advance(bits)?;
    Ok(InstructionOutcome::Continue)
}

pub(super) fn bs_test_unit(
    process: &Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, unit) = match operands {
        [fail, context, unit] => (fail, context, unit),
        _ => return Err(ExecError::InvalidOperand("bs_test_unit operands")),
    };
    let unit = core::operand_usize(unit, "bs_test_unit unit")?;
    if unit == 0 {
        return Err(ExecError::Badarg);
    }
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    if context.remaining_bits().is_multiple_of(unit) {
        Ok(InstructionOutcome::Continue)
    } else {
        jump_label(module, fail)
    }
}

pub(super) fn bs_get_float(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, size, unit, flags, destination) =
        parse_get_operands(operands, "bs_get_float2")?;
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let size_bits = segment_bits(size, unit)?;
    let Some(term) = get_float_term(process, context, size_bits, flags)? else {
        return jump_label(module, fail);
    };
    core::write_term(process, destination, term)?;
    context.advance(size_bits)?;
    Ok(InstructionOutcome::Continue)
}

pub(super) fn bs_get_tail(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, destination) = match operands {
        [fail, context, _live, destination] => (fail, context, destination),
        _ => return Err(ExecError::InvalidOperand("bs_get_tail operands")),
    };
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let remaining = context.remaining_bits();
    let Some(binary) = copy_binary_segment(process, context, remaining)? else {
        return jump_label(module, fail);
    };
    core::write_term(process, destination, binary)?;
    context.set_position_bits(context.total_bits());
    Ok(InstructionOutcome::Continue)
}

pub(super) fn bs_get_position(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (context, destination) = match operands {
        [context, destination, _live] => (context, destination),
        _ => return Err(ExecError::InvalidOperand("bs_get_position operands")),
    };
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let position = i64::try_from(context.position_bits()).map_err(|_| ExecError::Badarg)?;
    let term = Term::try_small_int(position).ok_or(ExecError::Badarg)?;
    core::write_term(process, destination, term)?;
    Ok(InstructionOutcome::Continue)
}

pub(super) fn bs_set_position(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (context, source) = match operands {
        [context, source] => (context, source),
        _ => return Err(ExecError::InvalidOperand("bs_set_position operands")),
    };
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let source = core::read_term(process, module, source)?;
    let position = source
        .as_small_int()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(ExecError::Badarg)?;
    if position > context.total_bits() {
        return Err(ExecError::Badarg);
    }
    context.set_position_bits(position);
    Ok(InstructionOutcome::Continue)
}

#[derive(Copy, Clone)]
pub(super) struct MatchContext {
    ptr: *mut u64,
}

impl MatchContext {
    pub(super) fn new(term: Term) -> Option<Self> {
        let ptr = term.heap_ptr()? as *mut u64;
        if boxed_tag(ptr) == Some(BoxedTag::MatchContext) {
            Some(Self { ptr })
        } else {
            None
        }
    }

    pub(super) fn position_bits(self) -> usize {
        read_word(self.ptr, 1) as usize
    }

    pub(super) fn set_position_bits(self, bits: usize) {
        write_word(self.ptr, 1, bits as u64);
    }

    pub(super) fn total_bits(self) -> usize {
        read_word(self.ptr, 2) as usize
    }

    pub(super) fn source(self) -> Option<Binary> {
        Binary::new(Term::from_raw(read_word(self.ptr, 3)))
    }

    pub(super) fn remaining_bits(self) -> usize {
        self.total_bits().saturating_sub(self.position_bits())
    }

    pub(super) fn has_bits(self, bits: usize) -> bool {
        self.position_bits()
            .checked_add(bits)
            .is_some_and(|end| end <= self.total_bits())
    }

    pub(super) fn advance(self, bits: usize) -> Result<(), ExecError> {
        let position = self
            .position_bits()
            .checked_add(bits)
            .ok_or(ExecError::Badarg)?;
        if position > self.total_bits() {
            return Err(ExecError::Badarg);
        }
        self.set_position_bits(position);
        Ok(())
    }

    pub(super) fn slice(self, bits: usize) -> Option<&'static [u8]> {
        if !bits.is_multiple_of(u8::BITS as usize)
            || !self.position_bits().is_multiple_of(u8::BITS as usize)
        {
            return None;
        }
        let start = self.position_bits() / u8::BITS as usize;
        let len = bits / u8::BITS as usize;
        let bytes = self.source()?.as_bytes();
        bytes.get(start..start + len)
    }
}

#[derive(Copy, Clone)]
pub(super) enum Endian {
    Big,
    Little,
}

impl Endian {
    pub(super) fn from_flags(flags: &Operand) -> Self {
        match flags {
            Operand::Unsigned(1) | Operand::Integer(1) => Self::Little,
            Operand::List(items) if items.iter().any(is_little_flag) => Self::Little,
            Operand::Unsigned(v) if v & 0x02 != 0 => Self::Little,
            Operand::Integer(v) if v & 0x02 != 0 => Self::Little,
            _ => Self::Big,
        }
    }
}

#[derive(Copy, Clone)]
pub(super) struct SegmentFlags {
    endian: Endian,
    signed: bool,
}

impl SegmentFlags {
    pub(super) fn from_flags(flags: &Operand) -> Self {
        let signed = match flags {
            Operand::Unsigned(v) => v & 0x04 != 0,
            Operand::Integer(v) => v & 0x04 != 0,
            Operand::List(items) => items.iter().any(is_signed_flag),
            _ => false,
        };
        Self {
            endian: Endian::from_flags(flags),
            signed,
        }
    }
}

fn is_signed_flag(flag: &Operand) -> bool {
    match flag {
        Operand::Unsigned(v) => v & 0x04 != 0,
        Operand::Integer(v) => v & 0x04 != 0,
        _ => false,
    }
}

pub(super) fn parse_get_operands<'a>(
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

pub(super) fn segment_bits(size: &Operand, unit: &Operand) -> Result<usize, ExecError> {
    let size = core::operand_usize(size, "segment size")?;
    let unit = core::operand_usize(unit, "segment unit")?;
    size.checked_mul(unit)
        .ok_or(ExecError::InvalidOperand("segment size"))
}

pub(super) fn decode_integer(bytes: &[u8], flags: SegmentFlags) -> Result<i64, ExecError> {
    if bytes.len() > std::mem::size_of::<i64>() {
        return Err(ExecError::Badarg);
    }
    let msb = match flags.endian {
        Endian::Big => bytes.first(),
        Endian::Little => bytes.last(),
    };
    let negative = flags.signed && msb.is_some_and(|byte| byte & 0x80 != 0);
    let fill = if negative { 0xff_u8 } else { 0x00_u8 };
    let mut full = [fill; 8];
    match flags.endian {
        Endian::Big => full[8 - bytes.len()..].copy_from_slice(bytes),
        Endian::Little => full[..bytes.len()].copy_from_slice(bytes),
    }
    Ok(match flags.endian {
        Endian::Big => u64::from_be_bytes(full) as i64,
        Endian::Little => u64::from_le_bytes(full) as i64,
    })
}

pub(super) fn literal_bytes<'a>(
    module: &'a Module,
    operand: &'a Operand,
    byte_len: usize,
) -> Result<&'a [u8], ExecError> {
    match operand {
        Operand::Literal(index) => match module.literals.get(*index) {
            Some(Literal::Binary(bytes) | Literal::String(bytes)) => bytes
                .get(..byte_len)
                .filter(|bytes| bytes.len() == byte_len)
                .ok_or(ExecError::Badarg),
            _ => Err(ExecError::Badarg),
        },
        offset => {
            let offset = core::operand_usize(offset, "string table offset")?;
            module
                .string_table
                .get(offset..offset + byte_len)
                .ok_or(ExecError::Badarg)
        }
    }
}

pub(super) fn get_integer_term(
    context: MatchContext,
    size: &Operand,
    unit: &Operand,
    flags: &Operand,
) -> Result<Option<Term>, ExecError> {
    let size_bits = segment_bits(size, unit)?;
    let segment_flags = SegmentFlags::from_flags(flags);
    if !size_bits.is_multiple_of(u8::BITS as usize)
        || !context.position_bits().is_multiple_of(u8::BITS as usize)
    {
        return Err(ExecError::Badarg);
    }
    if !context.has_bits(size_bits) {
        return Ok(None);
    }
    let bytes = context.slice(size_bits).ok_or(ExecError::Badarg)?;
    let value = decode_integer(bytes, segment_flags)?;
    Term::try_small_int(value)
        .ok_or(ExecError::Badarg)
        .map(Some)
}

pub(super) fn get_float_term(
    process: &mut Process,
    context: MatchContext,
    size_bits: usize,
    flags: &Operand,
) -> Result<Option<Term>, ExecError> {
    if !matches!(size_bits, 32 | 64) || !context.position_bits().is_multiple_of(u8::BITS as usize) {
        return Err(ExecError::Badarg);
    }
    if !context.has_bits(size_bits) {
        return Ok(None);
    }
    let bytes = context.slice(size_bits).ok_or(ExecError::Badarg)?;
    let value = match (size_bits, Endian::from_flags(flags)) {
        (32, Endian::Big) => f32::from_bits(u32::from_be_bytes(
            bytes.try_into().map_err(|_| ExecError::Badarg)?,
        )) as f64,
        (32, Endian::Little) => f32::from_bits(u32::from_le_bytes(
            bytes.try_into().map_err(|_| ExecError::Badarg)?,
        )) as f64,
        (64, Endian::Big) => f64::from_bits(u64::from_be_bytes(
            bytes.try_into().map_err(|_| ExecError::Badarg)?,
        )),
        (64, Endian::Little) => f64::from_bits(u64::from_le_bytes(
            bytes.try_into().map_err(|_| ExecError::Badarg)?,
        )),
        _ => return Err(ExecError::Badarg),
    };
    let words = 2;
    if process.heap().available() < words {
        return Err(ExecError::GcNeeded {
            requested: words,
            available: process.heap().available(),
        });
    }
    let ptr = process.heap_mut().alloc(words).map_err(ExecError::from)?;
    let heap = heap_slice(ptr, words);
    write_float(heap, value).ok_or(ExecError::Badarg).map(Some)
}

pub(super) fn copy_binary_segment(
    process: &mut Process,
    context: MatchContext,
    size_bits: usize,
) -> Result<Option<Term>, ExecError> {
    if !size_bits.is_multiple_of(u8::BITS as usize)
        || !context.position_bits().is_multiple_of(u8::BITS as usize)
    {
        return Err(ExecError::Badarg);
    }
    if !context.has_bits(size_bits) {
        return Ok(None);
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
    write_binary(heap, bytes).ok_or(ExecError::Badarg).map(Some)
}
