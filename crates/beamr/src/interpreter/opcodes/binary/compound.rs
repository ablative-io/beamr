use crate::error::ExecError;
use crate::interpreter::InstructionOutcome;
use crate::loader::decode::compact::Operand;
use crate::module::Module;
use crate::process::Process;

use super::core;
use super::jump_label;
use super::matching::{
    MatchContext, SegmentFlags, copy_binary_segment, decode_integer, get_float_term,
    get_integer_term, literal_bytes, segment_bits,
};

pub(super) fn bs_match(
    process: &mut Process,
    module: &Module,
    operands: &[Operand],
) -> Result<InstructionOutcome, ExecError> {
    if operands.len() < 3 {
        return Err(ExecError::InvalidOperand("bs_match operands"));
    }
    let fail = &operands[0];
    let context_term = core::read_term(process, module, &operands[1])?;
    let context = MatchContext::new(context_term).ok_or(ExecError::Badarg)?;
    let start_position = context.position_bits();
    let commands = bs_match_commands(&operands[2..]);
    let mut index = 0;
    let mut staged_writes = Vec::new();

    while index < commands.len() {
        let command = command_tag(&commands[index])?;
        index += 1;
        let succeeded = match command {
            BsMatchCommand::EnsureAtLeast => {
                let (size, unit) = take2(commands, &mut index, "bs_match ensure")?;
                context.has_bits(segment_bits(size, unit)?)
            }
            BsMatchCommand::EnsureExactly => {
                let stride = take1(commands, &mut index, "bs_match ensure_exactly")?;
                let stride = core::operand_usize(stride, "bs_match ensure_exactly stride")?;
                context.remaining_bits() == stride
            }
            BsMatchCommand::Integer => {
                let (_live, flags, size, unit, destination) =
                    take5(commands, &mut index, "bs_match integer")?;
                if let Some(term) = get_integer_term(context, size, unit, flags)? {
                    staged_writes.push((destination.clone(), term));
                    context.advance(segment_bits(size, unit)?)?;
                    true
                } else {
                    false
                }
            }
            BsMatchCommand::Float => {
                let (_live, flags, size, unit, destination) =
                    take5(commands, &mut index, "bs_match float")?;
                let size_bits = segment_bits(size, unit)?;
                if let Some(term) = get_float_term(process, context, size_bits, flags)? {
                    staged_writes.push((destination.clone(), term));
                    context.advance(size_bits)?;
                    true
                } else {
                    false
                }
            }
            BsMatchCommand::Binary => {
                let (_live, _flags, size, unit, destination) =
                    take5(commands, &mut index, "bs_match binary")?;
                let size_bits = segment_bits(size, unit)?;
                if let Some(term) = copy_binary_segment(process, context, size_bits)? {
                    staged_writes.push((destination.clone(), term));
                    context.advance(size_bits)?;
                    true
                } else {
                    false
                }
            }
            BsMatchCommand::Equal => {
                let (_live, bits, value) = take3(commands, &mut index, "bs_match =:=")?;
                match_exact_value(process, module, context, bits, value)?
            }
            BsMatchCommand::Skip => {
                let stride = take1(commands, &mut index, "bs_match skip")?;
                let bits = core::operand_usize(stride, "bs_match skip stride")?;
                if context.has_bits(bits) {
                    context.advance(bits)?;
                    true
                } else {
                    false
                }
            }
            BsMatchCommand::GetTail => {
                let tail = take_get_tail(commands, &mut index)?;
                if let Some(term) = copy_binary_segment(process, context, context.remaining_bits())?
                {
                    staged_writes.push((tail.destination.clone(), term));
                    context.set_position_bits(context.total_bits());
                    true
                } else {
                    false
                }
            }
        };
        if !succeeded {
            context.set_position_bits(start_position);
            return jump_label(module, fail);
        }
    }

    for (destination, term) in staged_writes {
        core::write_term(process, &destination, term)?;
    }
    Ok(InstructionOutcome::Continue)
}

fn bs_match_commands(operands: &[Operand]) -> &[Operand] {
    match operands {
        [Operand::List(commands)] => commands,
        [_z, _arity, Operand::List(commands)] => commands,
        [_z, _arity, _count, Operand::List(commands)] => commands,
        commands => commands,
    }
}

#[derive(Copy, Clone)]
enum BsMatchCommand {
    EnsureAtLeast,
    EnsureExactly,
    Integer,
    Float,
    Binary,
    Equal,
    Skip,
    GetTail,
}

fn command_tag(operand: &Operand) -> Result<BsMatchCommand, ExecError> {
    match operand {
        Operand::Unsigned(0) | Operand::Integer(0) => Ok(BsMatchCommand::EnsureAtLeast),
        Operand::Unsigned(1) | Operand::Integer(1) => Ok(BsMatchCommand::Integer),
        Operand::Unsigned(2) | Operand::Integer(2) => Ok(BsMatchCommand::Binary),
        Operand::Unsigned(3) | Operand::Integer(3) => Ok(BsMatchCommand::Equal),
        Operand::Unsigned(4) | Operand::Integer(4) => Ok(BsMatchCommand::Skip),
        Operand::Unsigned(5) | Operand::Integer(5) => Ok(BsMatchCommand::GetTail),
        Operand::Unsigned(6) | Operand::Integer(6) => Ok(BsMatchCommand::Float),
        Operand::Unsigned(7) | Operand::Integer(7) => Ok(BsMatchCommand::EnsureExactly),
        _ => Err(ExecError::InvalidOperand("bs_match command")),
    }
}

struct TailCommand<'a> {
    destination: &'a Operand,
}

fn take1<'a>(
    commands: &'a [Operand],
    index: &mut usize,
    context: &'static str,
) -> Result<&'a Operand, ExecError> {
    let value = commands
        .get(*index)
        .ok_or(ExecError::InvalidOperand(context))?;
    *index += 1;
    Ok(value)
}

fn take2<'a>(
    commands: &'a [Operand],
    index: &mut usize,
    context: &'static str,
) -> Result<(&'a Operand, &'a Operand), ExecError> {
    let a = take1(commands, index, context)?;
    let b = take1(commands, index, context)?;
    Ok((a, b))
}

fn take3<'a>(
    commands: &'a [Operand],
    index: &mut usize,
    context: &'static str,
) -> Result<(&'a Operand, &'a Operand, &'a Operand), ExecError> {
    let a = take1(commands, index, context)?;
    let b = take1(commands, index, context)?;
    let c = take1(commands, index, context)?;
    Ok((a, b, c))
}

fn take5<'a>(
    commands: &'a [Operand],
    index: &mut usize,
    context: &'static str,
) -> Result<
    (
        &'a Operand,
        &'a Operand,
        &'a Operand,
        &'a Operand,
        &'a Operand,
    ),
    ExecError,
> {
    let a = take1(commands, index, context)?;
    let b = take1(commands, index, context)?;
    let c = take1(commands, index, context)?;
    let d = take1(commands, index, context)?;
    let e = take1(commands, index, context)?;
    Ok((a, b, c, d, e))
}

fn take_get_tail<'a>(
    commands: &'a [Operand],
    index: &mut usize,
) -> Result<TailCommand<'a>, ExecError> {
    let first = take1(commands, index, "bs_match get_tail")?;
    let second = take1(commands, index, "bs_match get_tail")?;
    let destination = if commands.get(*index).is_some() && is_context_operand(second) {
        take1(commands, index, "bs_match get_tail")?
    } else {
        let _live = first;
        second
    };
    Ok(TailCommand { destination })
}

fn is_context_operand(operand: &Operand) -> bool {
    matches!(
        operand,
        Operand::X(_) | Operand::Y(_) | Operand::TypedRegister { .. }
    )
}

fn match_exact_value(
    process: &Process,
    module: &Module,
    context: MatchContext,
    bits: &Operand,
    value: &Operand,
) -> Result<bool, ExecError> {
    let bits = core::operand_usize(bits, "bs_match =:= bits")?;
    if !bits.is_multiple_of(u8::BITS as usize)
        || !context.position_bits().is_multiple_of(u8::BITS as usize)
    {
        return Err(ExecError::Badarg);
    }
    if !context.has_bits(bits) {
        return Ok(false);
    }
    let bytes = context.slice(bits).ok_or(ExecError::Badarg)?;
    let matches = if let Ok(expected) = literal_bytes(module, value, bytes.len()) {
        bytes == expected
    } else {
        let expected = core::read_term(process, module, value)?;
        let expected = expected.as_small_int().ok_or(ExecError::Badarg)?;
        decode_integer(bytes, SegmentFlags::from_flags(&Operand::Atom(None)))? == expected
    };
    if matches {
        context.advance(bits)?;
    }
    Ok(matches)
}
