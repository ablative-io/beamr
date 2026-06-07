use crate::error::ExecError;
use crate::interpreter::InstructionOutcome;
use crate::loader::decode::compact::Operand;
use crate::module::Module;
use crate::process::Process;
use crate::term::Term;

use super::core;
use super::jump_label;
use super::matching::{Endian, MatchContext};

type DecodeResult = Result<Option<(u32, usize)>, ExecError>;

pub(super) fn bs_get_utf8(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    get_utf(
        process,
        module,
        operands,
        "bs_get_utf8 operands",
        decode_utf8,
    )
}

pub(super) fn bs_skip_utf8(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    skip_utf(
        process,
        module,
        operands,
        "bs_skip_utf8 operands",
        decode_utf8,
    )
}

pub(super) fn bs_get_utf16(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, flags, destination) = get_operands(operands, "bs_get_utf16 operands")?;
    get_utf_with_flags(
        process,
        module,
        fail,
        context,
        flags,
        destination,
        decode_utf16,
    )
}

pub(super) fn bs_skip_utf16(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, flags) = skip_operands(operands, "bs_skip_utf16 operands")?;
    skip_utf_with_flags(process, module, fail, context, flags, decode_utf16)
}

pub(super) fn bs_get_utf32(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, flags, destination) = get_operands(operands, "bs_get_utf32 operands")?;
    get_utf_with_flags(
        process,
        module,
        fail,
        context,
        flags,
        destination,
        decode_utf32,
    )
}

pub(super) fn bs_skip_utf32(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, flags) = skip_operands(operands, "bs_skip_utf32 operands")?;
    skip_utf_with_flags(process, module, fail, context, flags, decode_utf32)
}

fn get_utf(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
    operand_context: &'static str,
    decoder: fn(&[u8]) -> DecodeResult,
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, _flags, destination) = get_operands(operands, operand_context)?;
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let Some((codepoint, bits)) = decode_at_context(context, decoder)? else {
        return jump_label(module, fail);
    };
    write_codepoint(process, destination, codepoint)?;
    context.advance(bits)?;
    Ok(InstructionOutcome::Continue)
}

fn skip_utf(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
    operand_context: &'static str,
    decoder: fn(&[u8]) -> DecodeResult,
) -> Result<InstructionOutcome, ExecError> {
    let (fail, context, _flags) = skip_operands(operands, operand_context)?;
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let Some((_codepoint, bits)) = decode_at_context(context, decoder)? else {
        return jump_label(module, fail);
    };
    context.advance(bits)?;
    Ok(InstructionOutcome::Continue)
}

fn get_utf_with_flags(
    process: &mut Process,
    module: &Module,
    fail: &Operand,
    context: &Operand,
    flags: &Operand,
    destination: &Operand,
    decoder: fn(&[u8], Endian) -> DecodeResult,
) -> Result<InstructionOutcome, ExecError> {
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let endian = Endian::from_flags(flags);
    let Some((codepoint, bits)) = decode_at_context_with_endian(context, endian, decoder)? else {
        return jump_label(module, fail);
    };
    write_codepoint(process, destination, codepoint)?;
    context.advance(bits)?;
    Ok(InstructionOutcome::Continue)
}

fn skip_utf_with_flags(
    process: &mut Process,
    module: &Module,
    fail: &Operand,
    context: &Operand,
    flags: &Operand,
    decoder: fn(&[u8], Endian) -> DecodeResult,
) -> Result<InstructionOutcome, ExecError> {
    let context_term = core::read_term(process, module, context)?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let endian = Endian::from_flags(flags);
    let Some((_codepoint, bits)) = decode_at_context_with_endian(context, endian, decoder)? else {
        return jump_label(module, fail);
    };
    context.advance(bits)?;
    Ok(InstructionOutcome::Continue)
}

fn get_operands<'a>(
    operands: &'a [Operand],
    context: &'static str,
) -> Result<(&'a Operand, &'a Operand, &'a Operand, &'a Operand), ExecError> {
    match operands {
        [fail, match_context, _live, flags, destination] => {
            Ok((fail, match_context, flags, destination))
        }
        _ => Err(ExecError::InvalidOperand(context)),
    }
}

fn skip_operands<'a>(
    operands: &'a [Operand],
    context: &'static str,
) -> Result<(&'a Operand, &'a Operand, &'a Operand), ExecError> {
    match operands {
        [fail, match_context, _live, flags] => Ok((fail, match_context, flags)),
        [fail, match_context, flags] => Ok((fail, match_context, flags)),
        _ => Err(ExecError::InvalidOperand(context)),
    }
}

fn decode_at_context(context: MatchContext, decoder: fn(&[u8]) -> DecodeResult) -> DecodeResult {
    if !context.position_bits().is_multiple_of(u8::BITS as usize) {
        return Ok(None);
    }
    let start = context.position_bits() / u8::BITS as usize;
    let bytes = context.source().ok_or(ExecError::Badarg)?;
    let remaining = bytes.as_bytes().get(start..).ok_or(ExecError::Badarg)?;
    decoder(remaining)
}

fn decode_at_context_with_endian(
    context: MatchContext,
    endian: Endian,
    decoder: fn(&[u8], Endian) -> DecodeResult,
) -> DecodeResult {
    if !context.position_bits().is_multiple_of(u8::BITS as usize) {
        return Ok(None);
    }
    let start = context.position_bits() / u8::BITS as usize;
    let bytes = context.source().ok_or(ExecError::Badarg)?;
    let remaining = bytes.as_bytes().get(start..).ok_or(ExecError::Badarg)?;
    decoder(remaining, endian)
}

fn write_codepoint(
    process: &mut Process,
    destination: &Operand,
    codepoint: u32,
) -> Result<(), ExecError> {
    let term = Term::try_small_int(i64::from(codepoint)).ok_or(ExecError::Badarg)?;
    core::write_term(process, destination, term)
}

fn decode_utf8(bytes: &[u8]) -> DecodeResult {
    let Some(&first) = bytes.first() else {
        return Ok(None);
    };
    let (needed, min, mut codepoint) = match first {
        0x00..=0x7f => return Ok(Some((u32::from(first), 8))),
        0xc2..=0xdf => (2, 0x80, u32::from(first & 0x1f)),
        0xe0..=0xef => (3, 0x800, u32::from(first & 0x0f)),
        0xf0..=0xf4 => (4, 0x10000, u32::from(first & 0x07)),
        _ => return Ok(None),
    };
    if bytes.len() < needed {
        return Ok(None);
    }
    for byte in &bytes[1..needed] {
        if byte & 0xc0 != 0x80 {
            return Ok(None);
        }
        codepoint = (codepoint << 6) | u32::from(byte & 0x3f);
    }
    if codepoint < min || !valid_codepoint(codepoint) {
        return Ok(None);
    }
    Ok(Some((codepoint, needed * u8::BITS as usize)))
}

fn decode_utf16(bytes: &[u8], endian: Endian) -> DecodeResult {
    let Some(first) = read_u16(bytes, endian) else {
        return Ok(None);
    };
    match first {
        0xd800..=0xdbff => {
            let Some(second) = bytes.get(2..).and_then(|tail| read_u16(tail, endian)) else {
                return Ok(None);
            };
            if !(0xdc00..=0xdfff).contains(&second) {
                return Ok(None);
            }
            let high = u32::from(first) - 0xd800;
            let low = u32::from(second) - 0xdc00;
            let codepoint = 0x10000 + ((high << 10) | low);
            if valid_codepoint(codepoint) {
                Ok(Some((codepoint, 32)))
            } else {
                Ok(None)
            }
        }
        0xdc00..=0xdfff => Ok(None),
        value => Ok(Some((u32::from(value), 16))),
    }
}

fn decode_utf32(bytes: &[u8], endian: Endian) -> DecodeResult {
    if bytes.len() < 4 {
        return Ok(None);
    }
    let value = match endian {
        Endian::Big => u32::from_be_bytes(bytes[..4].try_into().map_err(|_| ExecError::Badarg)?),
        Endian::Little => u32::from_le_bytes(bytes[..4].try_into().map_err(|_| ExecError::Badarg)?),
    };
    if valid_codepoint(value) {
        Ok(Some((value, 32)))
    } else {
        Ok(None)
    }
}

fn read_u16(bytes: &[u8], endian: Endian) -> Option<u16> {
    if bytes.len() < 2 {
        return None;
    }
    Some(match endian {
        Endian::Big => u16::from_be_bytes(bytes[..2].try_into().ok()?),
        Endian::Little => u16::from_le_bytes(bytes[..2].try_into().ok()?),
    })
}

fn valid_codepoint(codepoint: u32) -> bool {
    codepoint <= 0x10ffff && !(0xd800..=0xdfff).contains(&codepoint)
}
