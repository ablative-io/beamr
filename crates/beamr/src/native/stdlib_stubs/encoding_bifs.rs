//! Encoding-related native stubs for `binary` and `base64` modules.

use crate::atom::Atom;
use crate::native::ProcessContext;
use crate::term::Term;
use crate::term::binary_ref::BinaryRef;

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64_URLSAFE_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn bif_binary_encode_hex(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input] = args else {
        return Err(badarg());
    };
    let bytes = binary_bytes(*input)?;
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(nibble_to_hex(byte >> 4));
        out.push(nibble_to_hex(byte & 0x0f));
    }
    context.alloc_binary(&out)
}

pub fn bif_binary_decode_hex(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input] = args else {
        return Err(badarg());
    };
    let hex = binary_bytes(*input)?;
    if hex.len() % 2 != 0 {
        return Err(badarg());
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for pair in hex.chunks_exact(2) {
        let high = hex_value(pair[0])?;
        let low = hex_value(pair[1])?;
        out.push((high << 4) | low);
    }
    context.alloc_binary(&out)
}

pub fn bif_base64_encode(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input, options] = args else {
        return Err(badarg());
    };
    let (alphabet, padding) = base64_options(*options, context)?;
    let encoded = encode_base64(binary_bytes(*input)?, alphabet, padding);
    context.alloc_binary(encoded.as_bytes())
}

/// Decode OTP 26+ `base64:encode/2` options: either an alphabet atom
/// (`standard` | `urlsafe`) or a map `#{mode => standard | urlsafe,
/// padding => boolean()}` with both keys optional.
fn base64_options(
    options: Term,
    context: &ProcessContext,
) -> Result<(&'static [u8; 64], bool), Term> {
    if options.as_atom().is_some() {
        return Ok((base64_alphabet(options, context)?, true));
    }
    let map = crate::term::boxed::Map::new(options).ok_or_else(badarg)?;
    let mut alphabet: &'static [u8; 64] = BASE64_ALPHABET;
    let mut padding = true;
    for index in 0..map.len() {
        let key = map.key(index).ok_or_else(badarg)?;
        let value = map.value(index).ok_or_else(badarg)?;
        match atom_name(key, context)? {
            "mode" => alphabet = base64_alphabet(value, context)?,
            "padding" => {
                padding = match value.as_atom() {
                    Some(Atom::TRUE) => true,
                    Some(Atom::FALSE) => false,
                    _ => return Err(badarg()),
                }
            }
            _ => return Err(badarg()),
        }
    }
    Ok((alphabet, padding))
}

fn base64_alphabet(mode: Term, context: &ProcessContext) -> Result<&'static [u8; 64], Term> {
    match atom_name(mode, context)? {
        "standard" => Ok(BASE64_ALPHABET),
        "urlsafe" => Ok(BASE64_URLSAFE_ALPHABET),
        _ => Err(badarg()),
    }
}

pub fn bif_base64_decode(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input] = args else {
        return Err(badarg());
    };
    let decoded = decode_base64(binary_bytes(*input)?)?;
    context.alloc_binary(&decoded)
}

fn encode_base64(bytes: &[u8], alphabet: &[u8; 64], padding: bool) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        out.push(alphabet[(b0 >> 2) as usize] as char);
        out.push(alphabet[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(alphabet[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else if padding {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(alphabet[(b2 & 0x3f) as usize] as char);
        } else if padding {
            out.push('=');
        }
    }
    out
}

fn decode_base64(bytes: &[u8]) -> Result<Vec<u8>, Term> {
    if !bytes.len().is_multiple_of(4) {
        return Err(badarg());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (chunk_index, chunk) in bytes.chunks_exact(4).enumerate() {
        let last_chunk = chunk_index + 1 == bytes.len() / 4;
        let c0 = base64_value(chunk[0])?;
        let c1 = base64_value(chunk[1])?;
        let c2 = decode_base64_slot(chunk[2])?;
        let c3 = decode_base64_slot(chunk[3])?;
        if (!last_chunk && (c2.is_none() || c3.is_none())) || (c2.is_none() && c3.is_some()) {
            return Err(badarg());
        }
        let v2 = c2.unwrap_or(0);
        let v3 = c3.unwrap_or(0);
        out.push((c0 << 2) | (c1 >> 4));
        if c2.is_some() {
            out.push((c1 << 4) | (v2 >> 2));
        }
        if c3.is_some() {
            out.push((v2 << 6) | v3);
        }
    }
    Ok(out)
}

fn decode_base64_slot(byte: u8) -> Result<Option<u8>, Term> {
    if byte == b'=' {
        Ok(None)
    } else {
        base64_value(byte).map(Some)
    }
}

fn base64_value(byte: u8) -> Result<u8, Term> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(badarg()),
    }
}

fn nibble_to_hex(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        10..=15 => b'A' + (nibble - 10),
        _ => b'?',
    }
}

fn hex_value(byte: u8) -> Result<u8, Term> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(badarg()),
    }
}

fn atom_name<'a>(term: Term, context: &'a ProcessContext<'_>) -> Result<&'a str, Term> {
    let atom = term.as_atom().ok_or_else(badarg)?;
    if let Some(name) = context.atom_table().and_then(|table| table.resolve(atom)) {
        return Ok(name);
    }
    if atom == Atom::OK {
        Ok("ok")
    } else if atom == Atom::ERROR {
        Ok("error")
    } else if atom == Atom::TRUE {
        Ok("true")
    } else if atom == Atom::FALSE {
        Ok("false")
    } else {
        Err(badarg())
    }
}

fn binary_bytes(term: Term) -> Result<&'static [u8], Term> {
    BinaryRef::new(term)
        .map(|binary| binary.as_bytes())
        .ok_or_else(badarg)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}
