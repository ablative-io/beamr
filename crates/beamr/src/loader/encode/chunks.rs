//! Chunk-body encoders — exact inverses of the `decode::chunks` decoders.
//!
//! Each function returns a chunk body; the container writer frames it. The
//! `Code` and `LitT` bodies live in their own modules; everything else
//! (`AtU8`, `ImpT`, `ExpT`, `FunT`, `StrT`, `Line`) is here.

use crate::atom::Atom;
use crate::loader::decode::{ExportEntry, ImportEntry, LambdaEntry, LineInfo};

use super::compact::{AtomEncoder, push_signed, push_unsigned};
use super::container::EncodeError;

/// Encodes the `AtU8` chunk: an atom count followed by length-prefixed UTF-8
/// names. When every name fits a single length byte the positive-count form is
/// used (each name a `u8` length); otherwise the signed-negative count selects
/// the compact-length form the decoder reads for oversize names.
pub(crate) fn encode_atom_chunk(
    atoms: &[Atom],
    encoder: &AtomEncoder<'_>,
) -> Result<Vec<u8>, EncodeError> {
    let names = atoms
        .iter()
        .map(|atom| encoder.resolve(*atom))
        .collect::<Result<Vec<_>, _>>()?;
    let count = i32::try_from(names.len()).map_err(|_| EncodeError::ValueOutOfRange)?;
    let compact_lengths = names.iter().any(|name| name.len() > u8::MAX as usize);

    let mut out = Vec::new();
    if compact_lengths {
        out.extend_from_slice(
            &count
                .checked_neg()
                .ok_or(EncodeError::ValueOutOfRange)?
                .to_be_bytes(),
        );
    } else {
        out.extend_from_slice(&count.to_be_bytes());
    }
    for name in names {
        let bytes = name.as_bytes();
        if compact_lengths {
            push_unsigned(
                &mut out,
                0,
                u64::try_from(bytes.len()).map_err(|_| EncodeError::ValueOutOfRange)?,
            );
        } else {
            out.push(bytes.len() as u8);
        }
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

/// Encodes the `ImpT` chunk: count then `(module, function, arity)` atom-index
/// triples, all 1-based `u32` indices.
pub(crate) fn encode_import_chunk(
    imports: &[ImportEntry],
    encoder: &AtomEncoder<'_>,
) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    out.extend_from_slice(&count_u32(imports.len())?.to_be_bytes());
    for import in imports {
        out.extend_from_slice(&encoder.index_of(import.module)?.to_be_bytes());
        out.extend_from_slice(&encoder.index_of(import.function)?.to_be_bytes());
        out.extend_from_slice(&u32::from(import.arity).to_be_bytes());
    }
    Ok(out)
}

/// Encodes the `ExpT` chunk: count then `(function, arity, label)` triples.
pub(crate) fn encode_export_chunk(
    exports: &[ExportEntry],
    encoder: &AtomEncoder<'_>,
) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    out.extend_from_slice(&count_u32(exports.len())?.to_be_bytes());
    for export in exports {
        out.extend_from_slice(&encoder.index_of(export.function)?.to_be_bytes());
        out.extend_from_slice(&u32::from(export.arity).to_be_bytes());
        out.extend_from_slice(&export.label.to_be_bytes());
    }
    Ok(out)
}

/// Encodes the `FunT` chunk: count then six `u32` fields per lambda. The decoder
/// discards the `index` and `old_uniq` fields and recomputes the stable
/// `unique_id` from the module/function/arity/free-count, so this writer fills
/// `index` with the entry position and `old_uniq` with zero.
pub(crate) fn encode_lambda_chunk(
    lambdas: &[LambdaEntry],
    encoder: &AtomEncoder<'_>,
) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    out.extend_from_slice(&count_u32(lambdas.len())?.to_be_bytes());
    for (position, lambda) in lambdas.iter().enumerate() {
        out.extend_from_slice(&encoder.index_of(lambda.function)?.to_be_bytes());
        out.extend_from_slice(&u32::from(lambda.arity).to_be_bytes());
        out.extend_from_slice(&lambda.label.to_be_bytes());
        out.extend_from_slice(&count_u32(position)?.to_be_bytes());
        out.extend_from_slice(&lambda.num_free.to_be_bytes());
        out.extend_from_slice(&0_u32.to_be_bytes());
    }
    Ok(out)
}

/// Encodes the `StrT` chunk: the raw string-table bytes, verbatim.
pub(crate) fn encode_string_chunk(string_table: &[u8]) -> Vec<u8> {
    string_table.to_vec()
}

/// Encodes the `Line` chunk. The 20-byte header's version/flags/instruction and
/// filename counts are reconstructed from thin air (the decoder discards them);
/// only `num_lines` is load-bearing. Line items track the current file: a change
/// emits a tag-2 (atom) file reference followed by a tag-1 line number, an
/// unchanged file emits the tag-1 line number alone.
pub(crate) fn encode_line_chunk(line_info: &[LineInfo]) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    out.extend_from_slice(&0_u32.to_be_bytes()); // version
    out.extend_from_slice(&0_u32.to_be_bytes()); // flags
    out.extend_from_slice(&count_u32(line_info.len())?.to_be_bytes()); // num_line_instrs
    out.extend_from_slice(&count_u32(line_info.len())?.to_be_bytes()); // num_lines
    out.extend_from_slice(&0_u32.to_be_bytes()); // num_fnames

    let mut current_file = 0_u32;
    for info in line_info {
        if info.file != current_file {
            push_unsigned(&mut out, 2, u64::from(info.file));
            current_file = info.file;
        }
        push_signed(&mut out, 1, i64::from(info.line));
    }
    Ok(out)
}

fn count_u32(value: usize) -> Result<u32, EncodeError> {
    u32::try_from(value).map_err(|_| EncodeError::ValueOutOfRange)
}
