//! Compact term operand encoder.
//!
//! Exact inverse of [`super::super::decode::compact`]'s `read_tagged_value` /
//! `read_operand`: 1-byte, 2-byte and multi-byte (extended) packings for
//! tagged values, plus the extended subtags (list, float register, allocation
//! list, literal, typed register). Every byte this module writes is chosen so
//! the decoder reads back the identical [`Operand`].
//!
//! Oversized (>8-byte) integers never appear as compact operands — the decoder
//! diverts them into the constant pool as `Operand::Literal`, so this encoder
//! emits them as a literal reference and the value itself lives in `LitT`.

use std::collections::HashMap;

use crate::atom::{Atom, AtomTable};
use crate::loader::decode::{Allocation, Operand};

use super::container::EncodeError;

/// Resolves module atoms to their 1-based index in the emitted `AtU8` table.
///
/// The decoder reads an atom operand as `index - 1` into the module atom
/// vector, so the encoder maps each atom back to `position + 1`. When the same
/// atom appears at several positions the first is used; the operand decodes to
/// the identical interned atom regardless of which index carries it.
pub(crate) struct AtomEncoder<'a> {
    indices: HashMap<Atom, u32>,
    atom_table: &'a AtomTable,
}

impl<'a> AtomEncoder<'a> {
    pub(crate) fn new(atoms: &[Atom], atom_table: &'a AtomTable) -> Self {
        let mut indices = HashMap::with_capacity(atoms.len());
        for (position, atom) in atoms.iter().enumerate() {
            indices.entry(*atom).or_insert(position as u32 + 1);
        }
        Self {
            indices,
            atom_table,
        }
    }

    /// 1-based atom-table index for an atom operand.
    pub(crate) fn index_of(&self, atom: Atom) -> Result<u32, EncodeError> {
        self.indices
            .get(&atom)
            .copied()
            .ok_or(EncodeError::AtomNotInTable)
    }

    /// Resolves an atom to its interned name for chunk encoders.
    pub(crate) fn resolve(&self, atom: Atom) -> Result<&'a str, EncodeError> {
        self.atom_table
            .resolve(atom)
            .ok_or(EncodeError::AtomNotInTable)
    }
}

/// Encodes one operand, appending its bytes to `out`.
pub(crate) fn encode_operand(
    out: &mut Vec<u8>,
    operand: &Operand,
    atoms: &AtomEncoder<'_>,
) -> Result<(), EncodeError> {
    match operand {
        Operand::Unsigned(value) => push_unsigned(out, 0, *value),
        Operand::Integer(value) => push_signed(out, 1, *value),
        Operand::Atom(None) => push_unsigned(out, 2, 0),
        Operand::Atom(Some(atom)) => push_unsigned(out, 2, u64::from(atoms.index_of(*atom)?)),
        Operand::X(value) => push_unsigned(out, 3, u64::from(*value)),
        Operand::Y(value) => push_unsigned(out, 4, u64::from(*value)),
        Operand::Label(value) => push_unsigned(out, 5, u64::from(*value)),
        Operand::Character(value) => push_unsigned(out, 6, *value),
        Operand::Literal(index) => {
            push_unsigned(out, 7, 4);
            push_unsigned(out, 0, index_to_u64(*index)?);
        }
        Operand::FloatRegister(value) => {
            push_unsigned(out, 7, 2);
            push_unsigned(out, 0, u64::from(*value));
        }
        Operand::List(operands) => {
            push_unsigned(out, 7, 1);
            push_unsigned(out, 0, operands.len() as u64);
            for nested in operands {
                encode_operand(out, nested, atoms)?;
            }
        }
        Operand::Allocation(entries) => {
            push_unsigned(out, 7, 3);
            push_unsigned(out, 0, entries.len() as u64);
            for entry in entries {
                let (tag, value) = allocation_pair(entry);
                push_unsigned(out, 0, tag);
                push_unsigned(out, 0, value);
            }
        }
        Operand::TypedRegister {
            register: _,
            type_index,
        } => {
            // ⛔ REFUSE. Writing this operand emits a `#tr{}` whose type index
            // points into a `Type` chunk `encode_module` never produces, so the
            // result is a container OTP's tools cannot resolve — and beamr's own
            // loader accepts it happily, because it reads the index and ignores
            // the table. That combination is a silent fallback wearing a success
            // exit: the encoder reports Ok, the round-trip ratchet goes green,
            // and the bytes are wrong everywhere else.
            //
            // The guard is deliberately here, in the single operand writer, and
            // not in a separate walk over `Instruction`'s 64 variants: every
            // operand of every instruction — including nested `Operand::List`
            // members, which recurse through this function — must pass through
            // this match arm. A hand-written walk would have to be kept in step
            // with the enum by someone remembering to.
            return Err(EncodeError::TypedRegisterWithoutTypeChunk {
                instruction_index: None,
                type_index: *type_index,
            });
        }
    }
    Ok(())
}

fn allocation_pair(entry: &Allocation) -> (u64, u64) {
    match entry {
        Allocation::Words(value) => (0, *value),
        Allocation::Floats(value) => (1, *value),
        Allocation::Funs(value) => (2, *value),
        Allocation::Unknown { tag, value } => (*tag, *value),
    }
}

fn index_to_u64(index: usize) -> Result<u64, EncodeError> {
    u64::try_from(index).map_err(|_| EncodeError::ValueOutOfRange)
}

/// Emits a compact value with an unsigned payload under the given 3-bit tag.
pub(crate) fn push_unsigned(out: &mut Vec<u8>, tag: u8, value: u64) {
    if value <= 0x0F {
        out.push(((value as u8) << 4) | tag);
    } else if value <= 0x7FF {
        let high = (value >> 8) as u8;
        out.push((high << 5) | 0x08 | tag);
        out.push((value & 0xFF) as u8);
    } else {
        push_extended(out, tag, &minimal_unsigned_be(value));
    }
}

/// Emits a compact value with a signed payload (tag 1). Positive values below
/// 2048 reuse the short forms; everything else takes a two's-complement
/// extended encoding whose width preserves the sign bit.
pub(crate) fn push_signed(out: &mut Vec<u8>, tag: u8, value: i64) {
    if (0..=0x0F).contains(&value) {
        out.push(((value as u8) << 4) | tag);
    } else if (0..=0x7FF).contains(&value) {
        let high = (value >> 8) as u8;
        out.push((high << 5) | 0x08 | tag);
        out.push((value & 0xFF) as u8);
    } else {
        push_extended(out, tag, &minimal_signed_be(value));
    }
}

/// Writes the extended (multi-byte) header + payload for a big-endian byte run.
fn push_extended(out: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
    let count = bytes.len();
    if count <= 8 {
        // descriptor in 0..=6 selects a byte count of descriptor + 2.
        let descriptor = (count - 2) as u8;
        out.push((descriptor << 5) | 0x18 | tag);
    } else {
        // descriptor 7 means the byte count follows as a compact unsigned:
        // byte_count = extra_len + 9.
        out.push((7 << 5) | 0x18 | tag);
        push_unsigned(out, 0, (count - 9) as u64);
    }
    out.extend_from_slice(bytes);
}

/// Minimal big-endian bytes (>= 2, for the extended form) of an unsigned value.
fn minimal_unsigned_be(value: u64) -> Vec<u8> {
    let significant = 8 - (value.leading_zeros() / 8) as usize;
    let count = significant.max(2);
    value.to_be_bytes()[8 - count..].to_vec()
}

/// Minimal big-endian two's-complement bytes (>= 2) of a signed value, chosen so
/// the top bit of the first byte carries the sign the decoder expects.
fn minimal_signed_be(value: i64) -> Vec<u8> {
    let mut count = 1;
    while count < 8 && !fits_signed(value, count) {
        count += 1;
    }
    let count = count.max(2);
    value.to_be_bytes()[8 - count..].to_vec()
}

fn fits_signed(value: i64, bytes: usize) -> bool {
    let bits = bytes * 8 - 1;
    let min = -(1_i128 << bits);
    let max = (1_i128 << bits) - 1;
    (min..=max).contains(&i128::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::decode::Literal;
    use crate::loader::decode::compact::CompactDecoder;

    /// Round-trips one operand through the real decoder against `atoms`. A pool
    /// of placeholder literals backs any `Operand::Literal` bounds check.
    fn round_trip(operand: &Operand, atoms: &[Atom], atom_table: &AtomTable) -> Operand {
        let encoder = AtomEncoder::new(atoms, atom_table);
        let mut bytes = Vec::new();
        encode_operand(&mut bytes, operand, &encoder).expect("operand encodes");
        let literals = vec![Literal::Nil; 6000];
        let mut decoder = CompactDecoder::new(&bytes, atoms, &literals);
        let decoded = decoder.read_operand().expect("operand decodes");
        assert!(decoder.is_empty(), "all bytes consumed");
        decoded
    }

    fn table() -> AtomTable {
        AtomTable::with_common_atoms()
    }

    /// Encodes one operand and returns the error, asserting the encoder refused.
    fn refusal(operand: &Operand) -> EncodeError {
        let atom_table = table();
        let encoder = AtomEncoder::new(&[], &atom_table);
        let mut bytes = Vec::new();
        encode_operand(&mut bytes, operand, &encoder).expect_err("encoder must refuse this operand")
    }

    /// #95. A typed register names a `Type` chunk entry, and `encode_module`
    /// emits no `Type` chunk — so writing the operand produces a `Code` chunk
    /// referencing a table that is not in the container. beamr's own loader
    /// reads the index and ignores the table, so this round-tripped green while
    /// being unreadable by every other BEAM tool.
    #[test]
    fn typed_registers_are_refused_by_name() {
        let error = refusal(&Operand::TypedRegister {
            register: Box::new(Operand::X(4)),
            type_index: 12,
        });
        assert_eq!(
            error,
            EncodeError::TypedRegisterWithoutTypeChunk {
                instruction_index: None,
                type_index: 12,
            }
        );
        // "Loudly and by name": the message must carry the offending index, or
        // a reader has to go find it themselves.
        let rendered = error.to_string();
        assert!(
            rendered.contains("12") && rendered.contains("Type"),
            "refusal must name the Type chunk entry, got: {rendered}"
        );
    }

    /// The guard lives in the single operand writer precisely so nesting is
    /// covered by construction. This proves the recursion rather than trusting
    /// it: a typed register buried in a list must refuse exactly the same way.
    #[test]
    fn typed_registers_nested_in_a_list_are_refused() {
        let error = refusal(&Operand::List(vec![
            Operand::X(1),
            Operand::TypedRegister {
                register: Box::new(Operand::Y(2)),
                type_index: 7,
            },
        ]));
        assert_eq!(
            error,
            EncodeError::TypedRegisterWithoutTypeChunk {
                instruction_index: None,
                type_index: 7,
            }
        );
    }

    /// Positive control for both refusal tests: the surrounding shapes encode
    /// fine, so the refusals above are caused by the typed register and not by
    /// the operand wrapper or an unrelated encoder fault.
    #[test]
    fn the_refusal_control_encodes_when_the_typed_register_is_absent() {
        let atoms = table();
        let plain = Operand::List(vec![Operand::X(1), Operand::Y(2)]);
        assert_eq!(round_trip(&plain, &[], &atoms), plain);
        assert_eq!(round_trip(&Operand::X(4), &[], &atoms), Operand::X(4));
    }

    #[test]
    fn small_and_wide_registers_round_trip() {
        let atoms = table();
        for value in [0_u32, 15, 16, 2047, 2048, 65535, 100_000, u32::MAX] {
            assert_eq!(
                round_trip(&Operand::X(value), &[], &atoms),
                Operand::X(value)
            );
            assert_eq!(
                round_trip(&Operand::Y(value), &[], &atoms),
                Operand::Y(value)
            );
        }
    }

    #[test]
    fn unsigned_boundaries_round_trip() {
        let atoms = table();
        for value in [0_u64, 15, 16, 2047, 2048, 0x7FFF, 0x8000, i64::MAX as u64] {
            assert_eq!(
                round_trip(&Operand::Unsigned(value), &[], &atoms),
                Operand::Unsigned(value)
            );
        }
    }

    #[test]
    fn negative_and_large_integers_round_trip() {
        let atoms = table();
        for value in [
            -1_i64,
            -16,
            -2048,
            -40000,
            40000,
            i32::MIN as i64,
            crate::term::Term::SMALL_INT_MIN,
            crate::term::Term::SMALL_INT_MAX,
        ] {
            assert_eq!(
                round_trip(&Operand::Integer(value), &[], &atoms),
                Operand::Integer(value)
            );
        }
    }

    #[test]
    fn atom_index_width_boundaries_round_trip() {
        let atom_table = table();
        // Build an atom vector long enough that indices span the 1-byte,
        // 2-byte, and extended compact-value forms.
        let atoms: Vec<Atom> = (0..3000)
            .map(|n| atom_table.intern(&format!("atom_{n}")))
            .collect();
        // index 14 -> value 15 (1 byte); 2046 -> 2047 (2 byte); 2047 -> 2048 (ext).
        for position in [0_usize, 14, 15, 2046, 2047, 2999] {
            let operand = Operand::Atom(Some(atoms[position]));
            assert_eq!(round_trip(&operand, &atoms, &atom_table), operand);
        }
        assert_eq!(
            round_trip(&Operand::Atom(None), &atoms, &atom_table),
            Operand::Atom(None)
        );
    }

    #[test]
    fn extended_operands_round_trip() {
        let atoms = table();
        let operands = [
            Operand::Literal(0),
            Operand::Literal(5000),
            Operand::FloatRegister(0),
            Operand::FloatRegister(15),
            Operand::Character(0x1F600),
            Operand::List(vec![Operand::X(1), Operand::Integer(-7), Operand::Y(3)]),
            Operand::Allocation(vec![
                Allocation::Words(2),
                Allocation::Floats(1),
                Allocation::Funs(4),
                Allocation::Unknown { tag: 9, value: 7 },
            ]),
        ];
        for operand in &operands {
            assert_eq!(round_trip(operand, &[], &atoms), *operand);
        }
    }
}
