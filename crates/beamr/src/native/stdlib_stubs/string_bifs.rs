//! Erlang `string` module native stubs for binary string inputs.

use crate::atom::Atom;
use crate::native::ProcessContext;
use crate::term::Term;
use crate::term::binary::Binary;
use crate::term::boxed::write_cons;

pub fn bif_length(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [input] = args else {
        return Err(badarg());
    };
    let len = binary_bytes(*input)?.len();
    i64::try_from(len)
        .ok()
        .and_then(Term::try_small_int)
        .ok_or_else(badarg)
}

pub fn bif_reverse(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [input] = args else {
        return Err(badarg());
    };
    let mut bytes = binary_bytes(*input)?.to_vec();
    bytes.reverse();
    make_binary(&bytes)
}

pub fn bif_lowercase(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [input] = args else {
        return Err(badarg());
    };
    let bytes: Vec<u8> = binary_bytes(*input)?
        .iter()
        .map(|byte| byte.to_ascii_lowercase())
        .collect();
    make_binary(&bytes)
}

pub fn bif_uppercase(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [input] = args else {
        return Err(badarg());
    };
    let bytes: Vec<u8> = binary_bytes(*input)?
        .iter()
        .map(|byte| byte.to_ascii_uppercase())
        .collect();
    make_binary(&bytes)
}

pub fn bif_trim(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input, direction] = args else {
        return Err(badarg());
    };
    let bytes = binary_bytes(*input)?;
    let direction = atom_name(*direction, context)?;
    let (mut start, mut end) = (0, bytes.len());

    if direction == "leading" || direction == "both" {
        while start < end && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
    } else if direction != "trailing" {
        return Err(badarg());
    }

    if direction == "trailing" || direction == "both" {
        while end > start && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
    }

    make_binary(&bytes[start..end])
}

pub fn bif_split(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input, pattern, option] = args else {
        return Err(badarg());
    };
    let input = binary_bytes(*input)?;
    let pattern = binary_bytes(*pattern)?;
    if pattern.is_empty() {
        return Err(badarg());
    }
    let option = atom_name(*option, context)?;
    let parts = match option {
        "all" => split_all(input, pattern),
        "leading" => split_once(input, pattern, false),
        "trailing" => split_once(input, pattern, true),
        _ => return Err(badarg()),
    };

    let mut terms = Vec::with_capacity(parts.len());
    for part in parts {
        terms.push(make_binary(part)?);
    }
    make_list(&terms)
}

pub fn bif_equal(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [left, right] = args else {
        return Err(badarg());
    };
    Ok(bool_term(binary_bytes(*left)? == binary_bytes(*right)?))
}

pub fn bif_is_empty(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [input] = args else {
        return Err(badarg());
    };
    Ok(bool_term(binary_bytes(*input)?.is_empty()))
}

fn split_all<'a>(input: &'a [u8], pattern: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut index = 0;
    while let Some(relative) = find_bytes(&input[index..], pattern) {
        let match_start = index + relative;
        parts.push(&input[index..match_start]);
        index = match_start + pattern.len();
    }
    parts.push(&input[index..]);
    parts
}

fn split_once<'a>(input: &'a [u8], pattern: &[u8], trailing: bool) -> Vec<&'a [u8]> {
    let found = if trailing {
        rfind_bytes(input, pattern)
    } else {
        find_bytes(input, pattern)
    };
    if let Some(index) = found {
        vec![&input[..index], &input[index + pattern.len()..]]
    } else {
        vec![input]
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn rfind_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

fn atom_name(term: Term, context: &ProcessContext) -> Result<&str, Term> {
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
    Binary::new(term).map(Binary::as_bytes).ok_or_else(badarg)
}

fn bool_term(value: bool) -> Term {
    Term::atom(if value { Atom::TRUE } else { Atom::FALSE })
}

fn make_binary(bytes: &[u8]) -> Result<Term, Term> {
    let data_words = crate::term::binary::packed_word_count(bytes.len());
    let heap: &mut [u64] = Box::leak(vec![0u64; 2 + data_words].into_boxed_slice());
    crate::term::binary::write_binary(heap, bytes).ok_or_else(badarg)
}

fn make_list(elements: &[Term]) -> Result<Term, Term> {
    let mut tail = Term::NIL;
    for element in elements.iter().rev() {
        let cell = Box::leak(Box::new([0u64; 2]));
        tail = write_cons(cell, *element, tail).ok_or_else(badarg)?;
    }
    Ok(tail)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}
