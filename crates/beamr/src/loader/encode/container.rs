//! IFF container writer + the public `encode_module` entry point.
//!
//! Frames chunk bodies into the `FOR1`/`BEAM` container (4-byte chunk headers,
//! 4-byte alignment padding) in a fixed canonical order and stamps the outer
//! size, mirroring `loader::parser::parse_beam_chunks`.

use std::error::Error;
use std::fmt;

use crate::atom::AtomTable;
use crate::loader::ParsedModule;

use super::chunks::{
    encode_atom_chunk, encode_export_chunk, encode_import_chunk, encode_lambda_chunk,
    encode_line_chunk, encode_string_chunk,
};
use super::code::encode_code_chunk;
use super::compact::AtomEncoder;
use super::literals::encode_literal_chunk;

/// A `.beam` module could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// An operand referenced an atom absent from the module's atom table, or an
    /// atom could not be resolved to a name.
    AtomNotInTable,
    /// A count, index, or length exceeded the width its chunk field allows.
    ValueOutOfRange,
    /// A `Literal::BigInteger` payload was not a sign byte plus magnitude.
    MalformedBigInteger,
    /// An instruction shape cannot be expressed (e.g. a `MakeFun` with an
    /// operand count matching neither `make_fun2` nor `make_fun3`).
    UnsupportedInstruction,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::AtomNotInTable => "operand atom is absent from the module atom table",
            Self::ValueOutOfRange => "value exceeds the width of its BEAM chunk field",
            Self::MalformedBigInteger => "big-integer literal is not a sign byte plus magnitude",
            Self::UnsupportedInstruction => "instruction shape cannot be encoded",
        };
        formatter.write_str(message)
    }
}

impl Error for EncodeError {}

/// Encodes decoded module data back into `.beam` container bytes.
///
/// The returned bytes decode to a [`ParsedModule`] equal to `module` (atoms,
/// instructions, imports, exports, lambdas, literals, strings, lines) when
/// loaded against the same `atom_table`.
pub fn encode_module(
    module: &ParsedModule,
    atom_table: &AtomTable,
) -> Result<Vec<u8>, EncodeError> {
    let encoder = AtomEncoder::new(&module.atoms, atom_table);

    // Canonical chunk order.
    //
    // `AtU8`, `Code`, `ImpT`, `ExpT` and `StrT` are emitted UNCONDITIONALLY,
    // even when empty. Our own loader treats an absent optional chunk as empty
    // (`load.rs` reads `StrT` with `unwrap_or_default`), so omitting them round
    // trips perfectly through beamr and is invisible to our own suite — but
    // OTP's `beam_disasm` hard-requires all five and refuses the module with
    // `{missing_chunk, _, "StrT"}` (or `"ExpT"` / `"ImpT"`) when one is absent.
    // A module nobody else's tools can read is a module we cannot investigate
    // with the ecosystem's instruments, and that cost has already been paid
    // once: a 7,828-module sweep silently skipped our module because
    // `beam_disasm` would not open it.
    //
    // The required set was measured, not assumed — each chunk was stripped in
    // turn from a working module and fed to `beam_disasm` (OTP 29). `Attr`,
    // `CInf`, `Dbgi`, `Docs`, `Line`, `LocT`, `Meta` and `Type` are genuinely
    // optional and stay conditional. `FunT` is likewise not required. `LitT`
    // remains conditional: stripping it from a module that HAS literals leaves
    // dangling references, so that arm of the experiment is a confound rather
    // than evidence that it is mandatory.
    let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    chunks.push((b"AtU8", encode_atom_chunk(&module.atoms, &encoder)?));
    chunks.push((b"Code", encode_code_chunk(&module.instructions, &encoder)?));
    chunks.push((b"ImpT", encode_import_chunk(&module.imports, &encoder)?));
    chunks.push((b"ExpT", encode_export_chunk(&module.exports, &encoder)?));
    if !module.lambdas.is_empty() {
        chunks.push((b"FunT", encode_lambda_chunk(&module.lambdas, &encoder)?));
    }
    if let Some(literal_chunk) = encode_literal_chunk(&module.literals, &encoder)? {
        chunks.push((b"LitT", literal_chunk));
    }
    chunks.push((b"StrT", encode_string_chunk(&module.string_table)));
    if !module.line_info.is_empty() {
        chunks.push((b"Line", encode_line_chunk(&module.line_info)?));
    }

    Ok(frame_container(&chunks))
}

/// Frames chunk bodies into the outer `FOR1 … BEAM` container.
fn frame_container(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"BEAM");
    for (tag, chunk) in chunks {
        body.extend_from_slice(*tag);
        // A chunk body never approaches 4 GiB in practice; the loader itself
        // reads the length as `u32`, so a wider value could not round-trip.
        body.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
        body.extend_from_slice(chunk);
        let padding = (4 - (chunk.len() % 4)) % 4;
        body.resize(body.len() + padding, 0);
    }

    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(b"FOR1");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}
