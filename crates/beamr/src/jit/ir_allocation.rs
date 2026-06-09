//! Heap allocation opcode lowering for the JIT compiler.

use crate::loader::decode::Operand;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{FuncRef, InstBuilder, MemFlags, Value, types};
use cranelift_frontend::FunctionBuilder;

use super::compiler::JitError;
use super::ir_common::{read_operand_term, write_operand_term};

const TERM_TAG_MASK: i64 = 0b111;
const BOXED_TAG: i64 = 0b100;
const LIST_TAG: i64 = 0b101;
const TUPLE_HEADER_TAG: i64 = 0x10;
const HEADER_TAG_BITS: i64 = 8;
const WORD_BYTES: usize = 8;

#[derive(Clone, Copy)]
pub(crate) struct AllocationHelpers {
    pub(crate) tuple: FuncRef,
    pub(crate) cons: FuncRef,
}

pub(crate) struct LoweringContext {
    pub(crate) register_file: Value,
    pub(crate) process: Value,
    pub(crate) deopt: cranelift_codegen::ir::Block,
}

pub(crate) fn lower_put_list(
    builder: &mut FunctionBuilder<'_>,
    context: LoweringContext,
    cons_helper: FuncRef,
    head: &Operand,
    tail: &Operand,
    destination: &Operand,
) -> Result<(), JitError> {
    let call = builder.ins().call(cons_helper, &[context.process]);
    let heap = builder.inst_results(call)[0];
    branch_to_deopt_if_null(builder, heap, context.deopt);
    let head_value = read_operand_term(builder, context.register_file, head)?;
    let tail_value = read_operand_term(builder, context.register_file, tail)?;
    builder
        .ins()
        .store(MemFlags::trusted(), head_value, heap, 0);
    builder
        .ins()
        .store(MemFlags::trusted(), tail_value, heap, WORD_BYTES as i32);
    let term = builder.ins().bor_imm(heap, LIST_TAG);
    write_operand_term(builder, context.register_file, destination, term)
}

pub(crate) fn lower_put_tuple2(
    builder: &mut FunctionBuilder<'_>,
    context: LoweringContext,
    tuple_helper: FuncRef,
    destination: &Operand,
    elements: &Operand,
) -> Result<(), JitError> {
    let Operand::List(elements) = elements else {
        return Err(tuple_elements_error(elements));
    };
    let arity = i64::try_from(elements.len()).map_err(|_| JitError::UnsupportedOperand {
        operand: format!("tuple arity {}", elements.len()),
    })?;
    let arity_value = builder.ins().iconst(types::I64, arity);
    let call = builder
        .ins()
        .call(tuple_helper, &[context.process, arity_value]);
    let heap = builder.inst_results(call)[0];
    branch_to_deopt_if_null(builder, heap, context.deopt);

    let header = (arity << HEADER_TAG_BITS) | TUPLE_HEADER_TAG;
    let header = builder.ins().iconst(types::I64, header);
    builder.ins().store(MemFlags::trusted(), header, heap, 0);
    for (index, element) in elements.iter().enumerate() {
        let value = read_operand_term(builder, context.register_file, element)?;
        let offset =
            i32::try_from((index + 1) * WORD_BYTES).map_err(|_| JitError::UnsupportedOperand {
                operand: format!("tuple element offset {index}"),
            })?;
        builder
            .ins()
            .store(MemFlags::trusted(), value, heap, offset);
    }

    let term = builder.ins().bor_imm(heap, BOXED_TAG);
    write_operand_term(builder, context.register_file, destination, term)
}

pub(crate) fn lower_get_list(
    builder: &mut FunctionBuilder<'_>,
    register_file: Value,
    source: &Operand,
    head: &Operand,
    tail: &Operand,
) -> Result<(), JitError> {
    let cons = read_cons_pointer(builder, register_file, source)?;
    let head_value = builder.ins().load(types::I64, MemFlags::trusted(), cons, 0);
    let tail_value = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), cons, WORD_BYTES as i32);
    write_operand_term(builder, register_file, head, head_value)?;
    write_operand_term(builder, register_file, tail, tail_value)
}

pub(crate) fn lower_get_hd(
    builder: &mut FunctionBuilder<'_>,
    register_file: Value,
    source: &Operand,
    destination: &Operand,
) -> Result<(), JitError> {
    let cons = read_cons_pointer(builder, register_file, source)?;
    let head = builder.ins().load(types::I64, MemFlags::trusted(), cons, 0);
    write_operand_term(builder, register_file, destination, head)
}

pub(crate) fn lower_get_tl(
    builder: &mut FunctionBuilder<'_>,
    register_file: Value,
    source: &Operand,
    destination: &Operand,
) -> Result<(), JitError> {
    let cons = read_cons_pointer(builder, register_file, source)?;
    let tail = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), cons, WORD_BYTES as i32);
    write_operand_term(builder, register_file, destination, tail)
}

pub(crate) fn lower_get_tuple_element(
    builder: &mut FunctionBuilder<'_>,
    register_file: Value,
    source: &Operand,
    index: usize,
    destination: &Operand,
) -> Result<(), JitError> {
    let tuple = read_boxed_pointer(builder, register_file, source)?;
    let offset = element_offset(index)?;
    let element = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), tuple, offset);
    write_operand_term(builder, register_file, destination, element)
}

pub(crate) fn tuple_root_operands(
    destination: &Operand,
    elements: &Operand,
) -> Result<Vec<Operand>, JitError> {
    let Operand::List(elements) = elements else {
        return Err(tuple_elements_error(elements));
    };
    let mut roots = Vec::with_capacity(elements.len() + 1);
    roots.extend(elements.iter().cloned());
    roots.push(destination.clone());
    Ok(roots)
}

fn branch_to_deopt_if_null(
    builder: &mut FunctionBuilder<'_>,
    pointer: Value,
    deopt: cranelift_codegen::ir::Block,
) {
    let is_null = builder.ins().icmp_imm(IntCC::Equal, pointer, 0);
    let continuation = builder.create_block();
    builder.ins().brif(is_null, deopt, &[], continuation, &[]);
    builder.switch_to_block(continuation);
}

fn tuple_elements_error(elements: &Operand) -> JitError {
    JitError::UnsupportedOperand {
        operand: format!("put_tuple2 elements must be a list, got {elements:?}"),
    }
}

fn read_cons_pointer(
    builder: &mut FunctionBuilder<'_>,
    register_file: Value,
    source: &Operand,
) -> Result<Value, JitError> {
    let term = read_operand_term(builder, register_file, source)?;
    Ok(builder.ins().band_imm(term, !TERM_TAG_MASK))
}

fn read_boxed_pointer(
    builder: &mut FunctionBuilder<'_>,
    register_file: Value,
    source: &Operand,
) -> Result<Value, JitError> {
    let term = read_operand_term(builder, register_file, source)?;
    Ok(builder.ins().band_imm(term, !TERM_TAG_MASK))
}

fn element_offset(index: usize) -> Result<i32, JitError> {
    let word_index = index
        .checked_add(1)
        .ok_or_else(|| JitError::UnsupportedOperand {
            operand: format!("tuple element offset {index}"),
        })?;
    let byte_offset =
        word_index
            .checked_mul(WORD_BYTES)
            .ok_or_else(|| JitError::UnsupportedOperand {
                operand: format!("tuple element offset {index}"),
            })?;
    i32::try_from(byte_offset).map_err(|_| JitError::UnsupportedOperand {
        operand: format!("tuple element offset {index}"),
    })
}
