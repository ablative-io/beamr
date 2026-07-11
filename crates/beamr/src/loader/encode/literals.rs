//! `LitT` literal-table encoder.
//!
//! ETF-encodes each loader [`Literal`] and packs the results into the `LitT`
//! chunk body: a 4-byte literal count, then per literal a 4-byte size and the
//! version-prefixed external term. The body is zlib-compressed and prefixed
//! with its uncompressed size, exactly as `decode::chunks::decode_literal_chunk`
//! expects.
//!
//! The type domain here is the loader's `Literal`, not a runtime `Term`, so the
//! encoder walks `Literal` directly. Tag encodings mirror `etf::tags`, staying
//! within the 16 tags `decode::etf` accepts on read.

use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::io::Write;

use crate::etf::tags;
use crate::loader::decode::Literal;

use super::compact::AtomEncoder;
use super::container::EncodeError;

/// Encodes the module's literal table into a `LitT` chunk body. Returns `None`
/// when there are no literals (the chunk is then omitted entirely).
pub(crate) fn encode_literal_chunk(
    literals: &[Literal],
    atoms: &AtomEncoder<'_>,
) -> Result<Option<Vec<u8>>, EncodeError> {
    if literals.is_empty() {
        return Ok(None);
    }

    let mut payload = Vec::new();
    payload.extend_from_slice(&u32_len(literals.len())?.to_be_bytes());
    for literal in literals {
        let mut term = vec![tags::VERSION];
        encode_literal(&mut term, literal, atoms)?;
        payload.extend_from_slice(&u32_len(term.len())?.to_be_bytes());
        payload.extend_from_slice(&term);
    }

    let uncompressed_size = u32_len(payload.len())?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&payload)
        .map_err(|_| EncodeError::CompressionFailed)?;
    let compressed = encoder
        .finish()
        .map_err(|_| EncodeError::CompressionFailed)?;

    let mut chunk = Vec::with_capacity(4 + compressed.len());
    chunk.extend_from_slice(&uncompressed_size.to_be_bytes());
    chunk.extend_from_slice(&compressed);
    Ok(Some(chunk))
}

/// Encodes one literal as an external term (without the leading version byte).
fn encode_literal(
    out: &mut Vec<u8>,
    literal: &Literal,
    atoms: &AtomEncoder<'_>,
) -> Result<(), EncodeError> {
    match literal {
        Literal::Integer(value) => encode_integer(out, *value),
        Literal::Float(value) => {
            out.push(tags::NEW_FLOAT_EXT);
            out.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        Literal::BigInteger(bytes) => encode_big_integer(out, bytes)?,
        Literal::Atom(atom) => encode_atom_name(out, atoms.resolve(*atom)?)?,
        Literal::Binary(bytes) => {
            out.push(tags::BINARY_EXT);
            out.extend_from_slice(&u32_len(bytes.len())?.to_be_bytes());
            out.extend_from_slice(bytes);
        }
        Literal::Tuple(elements) => {
            if let Ok(arity) = u8::try_from(elements.len()) {
                out.push(tags::SMALL_TUPLE_EXT);
                out.push(arity);
            } else {
                out.push(tags::LARGE_TUPLE_EXT);
                out.extend_from_slice(&u32_len(elements.len())?.to_be_bytes());
            }
            for element in elements {
                encode_literal(out, element, atoms)?;
            }
        }
        Literal::Nil => out.push(tags::NIL_EXT),
        Literal::List(elements, tail) => {
            out.push(tags::LIST_EXT);
            out.extend_from_slice(&u32_len(elements.len())?.to_be_bytes());
            for element in elements {
                encode_literal(out, element, atoms)?;
            }
            encode_literal(out, tail, atoms)?;
        }
        Literal::Map(pairs) => {
            out.push(tags::MAP_EXT);
            out.extend_from_slice(&u32_len(pairs.len())?.to_be_bytes());
            for (key, value) in pairs {
                encode_literal(out, key, atoms)?;
                encode_literal(out, value, atoms)?;
            }
        }
        Literal::String(bytes) => {
            let length = u16::try_from(bytes.len()).map_err(|_| EncodeError::ValueOutOfRange)?;
            out.push(tags::STRING_EXT);
            out.extend_from_slice(&length.to_be_bytes());
            out.extend_from_slice(bytes);
        }
        Literal::ExportFun {
            module,
            function,
            arity,
        } => {
            out.push(tags::EXPORT_EXT);
            encode_atom_name(out, atoms.resolve(*module)?)?;
            encode_atom_name(out, atoms.resolve(*function)?)?;
            encode_integer(out, i64::from(*arity));
        }
    }
    Ok(())
}

fn encode_integer(out: &mut Vec<u8>, value: i64) {
    if let Ok(byte) = u8::try_from(value) {
        out.push(tags::SMALL_INTEGER_EXT);
        out.push(byte);
    } else if let Ok(narrow) = i32::try_from(value) {
        out.push(tags::INTEGER_EXT);
        out.extend_from_slice(&narrow.to_be_bytes());
    } else {
        let negative = value.is_negative();
        let magnitude = value.unsigned_abs().to_le_bytes();
        let trimmed = trim_trailing_zeros(&magnitude);
        // A value wider than i32 fits within 8 magnitude bytes, so SMALL_BIG
        // (u8 length) always suffices here.
        out.push(tags::SMALL_BIG_EXT);
        out.push(trimmed.len() as u8);
        out.push(u8::from(negative));
        out.extend_from_slice(trimmed);
    }
}

/// Encodes a loader `BigInteger` (a sign byte followed by little-endian
/// magnitude bytes) as `SMALL_BIG_EXT` / `LARGE_BIG_EXT`. The magnitude bytes
/// are written verbatim — untrimmed — so the decoder reconstructs the identical
/// literal, including any values whose 8-byte magnitude exceeds `i64`.
fn encode_big_integer(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), EncodeError> {
    let (sign, magnitude) = bytes
        .split_first()
        .ok_or(EncodeError::MalformedBigInteger)?;
    if *sign > 1 {
        return Err(EncodeError::MalformedBigInteger);
    }
    if let Ok(length) = u8::try_from(magnitude.len()) {
        out.push(tags::SMALL_BIG_EXT);
        out.push(length);
    } else {
        out.push(tags::LARGE_BIG_EXT);
        out.extend_from_slice(&u32_len(magnitude.len())?.to_be_bytes());
    }
    out.push(*sign);
    out.extend_from_slice(magnitude);
    Ok(())
}

fn encode_atom_name(out: &mut Vec<u8>, name: &str) -> Result<(), EncodeError> {
    let bytes = name.as_bytes();
    if let Ok(length) = u8::try_from(bytes.len()) {
        out.push(tags::SMALL_ATOM_UTF8_EXT);
        out.push(length);
    } else {
        let length = u16::try_from(bytes.len()).map_err(|_| EncodeError::ValueOutOfRange)?;
        out.push(tags::ATOM_UTF8_EXT);
        out.extend_from_slice(&length.to_be_bytes());
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn trim_trailing_zeros(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(1, |index| index + 1);
    &bytes[..end]
}

fn u32_len(value: usize) -> Result<u32, EncodeError> {
    u32::try_from(value).map_err(|_| EncodeError::ValueOutOfRange)
}
