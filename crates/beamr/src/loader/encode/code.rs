//! `Code` chunk writer.
//!
//! Emits the 20-byte header (`sub_size = 16`, `instruction_set = 0`) followed by
//! the instruction stream. `opcode_max`, `label_count`, and `function_count` are
//! all derived from the instructions themselves — never hand-set — so the module
//! satisfies the loader's `label <= label_count` and `opcode <= opcode_max`
//! header consistency checks by construction.

use crate::loader::decode::Instruction;

use super::compact::AtomEncoder;
use super::container::EncodeError;
use super::opcodes::{encode_instruction, instruction_opcode};

const SUB_SIZE: u32 = 16;
const INSTRUCTION_SET: u32 = 0;

/// Encodes the `Code` chunk body for an instruction stream.
pub(crate) fn encode_code_chunk(
    instructions: &[Instruction],
    atoms: &AtomEncoder<'_>,
) -> Result<Vec<u8>, EncodeError> {
    let mut opcode_max = 0_u32;
    let mut label_count = 0_u32;
    let mut function_count = 0_u32;

    let mut body = Vec::new();
    for instruction in instructions {
        opcode_max = opcode_max.max(u32::from(instruction_opcode(instruction)?));
        if let Instruction::Label { label } = instruction {
            label_count = label_count.max(*label);
        }
        if matches!(instruction, Instruction::FuncInfo { .. }) {
            function_count += 1;
        }
        encode_instruction(&mut body, instruction, atoms)?;
    }

    let mut chunk = Vec::with_capacity(20 + body.len());
    chunk.extend_from_slice(&SUB_SIZE.to_be_bytes());
    chunk.extend_from_slice(&INSTRUCTION_SET.to_be_bytes());
    chunk.extend_from_slice(&opcode_max.to_be_bytes());
    chunk.extend_from_slice(&label_count.to_be_bytes());
    chunk.extend_from_slice(&function_count.to_be_bytes());
    chunk.extend_from_slice(&body);
    Ok(chunk)
}
