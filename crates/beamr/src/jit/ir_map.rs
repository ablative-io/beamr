//! Runtime-helper-backed lowering for BEAM map opcodes.

use crate::loader::decode::{MapOp, Operand};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    Block, FuncRef, InstBuilder, MemFlags, StackSlotData, StackSlotKind, Value, types,
};
use cranelift_frontend::FunctionBuilder;

use super::compiler::JitError;
use super::ir_common::{branch_to_fail_if, read_operand_term, write_operand_term};

const WORD_BYTES: usize = 8;

#[derive(Clone, Copy)]
pub(crate) struct MapHelpers {
    pub(crate) new: FuncRef,
    pub(crate) update: FuncRef,
    pub(crate) get: FuncRef,
    pub(crate) has_key: FuncRef,
}

#[derive(Clone, Copy)]
pub(crate) struct MapLoweringContext {
    pub(crate) register_file: Value,
    pub(crate) process: Value,
}

#[derive(Clone, Copy)]
pub(crate) struct PutMap<'a> {
    pub(crate) source: &'a Operand,
    pub(crate) destination: &'a Operand,
    pub(crate) pairs: &'a [Operand],
}

#[derive(Clone, Copy)]
pub(crate) struct GetMapElements<'a> {
    pub(crate) source: &'a Operand,
    pub(crate) pairs: &'a [Operand],
}

#[derive(Clone, Copy)]
pub(crate) struct HasMapFields<'a> {
    pub(crate) source: &'a Operand,
    pub(crate) keys: &'a [Operand],
}

pub(crate) fn translate_put_map_assoc(
    builder: &mut FunctionBuilder<'_>,
    context: MapLoweringContext,
    helpers: MapHelpers,
    op: PutMap<'_>,
    fail: Block,
) -> Result<(), JitError> {
    let source = read_operand_term(builder, context.register_file, op.source)?;
    let (pairs_ptr, pair_count) = stage_pairs(builder, context.register_file, op.pairs)?;
    let call = builder.ins().call(
        helpers.new,
        &[context.process, source, pairs_ptr, pair_count],
    );
    let result = builder.inst_results(call)[0];
    branch_to_fail_if_null(builder, result, fail);
    write_operand_term(builder, context.register_file, op.destination, result)
}

pub(crate) fn translate_put_map_exact(
    builder: &mut FunctionBuilder<'_>,
    context: MapLoweringContext,
    helpers: MapHelpers,
    op: PutMap<'_>,
    fail: Block,
) -> Result<(), JitError> {
    let source = read_operand_term(builder, context.register_file, op.source)?;
    let (pairs_ptr, pair_count) = stage_pairs(builder, context.register_file, op.pairs)?;
    let call = builder.ins().call(
        helpers.update,
        &[context.process, source, pairs_ptr, pair_count],
    );
    let result = builder.inst_results(call)[0];
    branch_to_fail_if_null(builder, result, fail);
    write_operand_term(builder, context.register_file, op.destination, result)
}

pub(crate) fn translate_get_map_elements(
    builder: &mut FunctionBuilder<'_>,
    context: MapLoweringContext,
    helpers: MapHelpers,
    op: GetMapElements<'_>,
    fail: Block,
) -> Result<(), JitError> {
    if !op.pairs.len().is_multiple_of(2) {
        return Err(JitError::UnsupportedOperand {
            operand: format!(
                "get_map_elements pairs must be even, got {}",
                op.pairs.len()
            ),
        });
    }

    let map = read_operand_term(builder, context.register_file, op.source)?;
    let mut found = Vec::with_capacity(op.pairs.len() / 2);
    for pair in op.pairs.chunks_exact(2) {
        let key = read_operand_term(builder, context.register_file, &pair[0])?;
        let call = builder.ins().call(helpers.get, &[map, key]);
        let results = builder.inst_results(call).to_vec();
        let missing = builder.ins().icmp_imm(IntCC::Equal, results[0], 0);
        branch_to_fail_if(builder, missing, fail);
        found.push((&pair[1], results[1]));
    }

    for (destination, value) in found {
        write_operand_term(builder, context.register_file, destination, value)?;
    }
    Ok(())
}

pub(crate) fn translate_has_map_fields(
    builder: &mut FunctionBuilder<'_>,
    context: MapLoweringContext,
    helpers: MapHelpers,
    op: HasMapFields<'_>,
    fail: Block,
) -> Result<(), JitError> {
    let map = read_operand_term(builder, context.register_file, op.source)?;
    for key in op.keys {
        let key = read_operand_term(builder, context.register_file, key)?;
        let call = builder.ins().call(helpers.has_key, &[map, key]);
        let present = builder.inst_results(call)[0];
        let missing = builder.ins().icmp_imm(IntCC::Equal, present, 0);
        branch_to_fail_if(builder, missing, fail);
    }
    Ok(())
}

pub(crate) fn parse_put_map_operands(
    operands: &[Operand],
) -> Result<(&Operand, PutMap<'_>), JitError> {
    let [fail, source, destination, _live, Operand::List(items)] = operands else {
        return Err(JitError::UnsupportedOperand {
            operand: format!("put_map operands {operands:?}"),
        });
    };
    if items.len() % 2 != 0 {
        return Err(JitError::UnsupportedOperand {
            operand: format!("put_map pairs must be even, got {}", items.len()),
        });
    }
    Ok((
        fail,
        PutMap {
            source,
            destination,
            pairs: items,
        },
    ))
}

pub(crate) fn parse_get_map_elements_operands(
    operands: &[Operand],
) -> Result<(&Operand, GetMapElements<'_>), JitError> {
    let [fail, source, Operand::List(items)] = operands else {
        return Err(JitError::UnsupportedOperand {
            operand: format!("get_map_elements operands {operands:?}"),
        });
    };
    if items.len() % 2 != 0 {
        return Err(JitError::UnsupportedOperand {
            operand: format!("get_map_elements pairs must be even, got {}", items.len()),
        });
    }
    Ok((
        fail,
        GetMapElements {
            source,
            pairs: items,
        },
    ))
}

pub(crate) fn parse_has_map_fields_operands(
    operands: &[Operand],
) -> Result<(&Operand, HasMapFields<'_>), JitError> {
    let [fail, source, Operand::List(keys)] = operands else {
        return Err(JitError::UnsupportedOperand {
            operand: format!("has_map_fields operands {operands:?}"),
        });
    };
    Ok((fail, HasMapFields { source, keys }))
}

pub(crate) fn map_allocation_roots(
    op: MapOp,
    operands: &[Operand],
) -> Result<Vec<Operand>, JitError> {
    match op {
        MapOp::PutMapAssoc | MapOp::PutMapExact => {
            let (_fail, parsed) = parse_put_map_operands(operands)?;
            let mut roots = Vec::with_capacity(parsed.pairs.len() + 2);
            roots.push(parsed.source.clone());
            roots.extend(parsed.pairs.iter().cloned());
            roots.push(parsed.destination.clone());
            Ok(roots)
        }
        MapOp::HasMapFields | MapOp::GetMapElements => Ok(Vec::new()),
    }
}

fn stage_pairs(
    builder: &mut FunctionBuilder<'_>,
    register_file: Value,
    pairs: &[Operand],
) -> Result<(Value, Value), JitError> {
    if !pairs.len().is_multiple_of(2) {
        return Err(JitError::UnsupportedOperand {
            operand: format!("map pairs must be even, got {}", pairs.len()),
        });
    }
    let bytes =
        pairs
            .len()
            .max(1)
            .checked_mul(WORD_BYTES)
            .ok_or_else(|| JitError::UnsupportedOperand {
                operand: format!("map pair stack bytes for {} terms", pairs.len()),
            })?;
    let bytes = u32::try_from(bytes).map_err(|_| JitError::UnsupportedOperand {
        operand: format!("map pair stack bytes for {} terms", pairs.len()),
    })?;
    let slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, bytes, 3));
    let slot_addr = builder.ins().stack_addr(types::I64, slot, 0);
    for (index, operand) in pairs.iter().enumerate() {
        let value = read_operand_term(builder, register_file, operand)?;
        let offset =
            i32::try_from(index * WORD_BYTES).map_err(|_| JitError::UnsupportedOperand {
                operand: format!("map pair stack offset {index}"),
            })?;
        builder
            .ins()
            .store(MemFlags::trusted(), value, slot_addr, offset);
    }
    let pair_count = i64::try_from(pairs.len() / 2).map_err(|_| JitError::UnsupportedOperand {
        operand: format!("map pair count {}", pairs.len() / 2),
    })?;
    Ok((slot_addr, builder.ins().iconst(types::I64, pair_count)))
}

fn branch_to_fail_if_null(builder: &mut FunctionBuilder<'_>, value: Value, fail: Block) {
    let is_null = builder.ins().icmp_imm(IntCC::Equal, value, 0);
    branch_to_fail_if(builder, is_null, fail);
}
