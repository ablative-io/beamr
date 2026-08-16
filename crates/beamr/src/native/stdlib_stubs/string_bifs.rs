//! Erlang `string` module native stubs for binary string inputs.
//!
//! These natives serve the real `gleam_stdlib.erl` bytecode (the gleam-level
//! FFI shadows were removed), so their contracts must match Erlang/OTP's
//! `string` module exactly: lengths, slices, reversal, and padding operate on
//! extended grapheme clusters, and case/trim operations use full Unicode
//! mappings rather than ASCII approximations.

use crate::atom::Atom;
use crate::native::ProcessContext;
use crate::term::Term;
use crate::term::binary_ref::BinaryRef;
use unicode_segmentation::UnicodeSegmentation;

pub fn bif_length(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [input] = args else {
        return Err(badarg());
    };
    let len = utf8_str(*input)?.graphemes(true).count();
    i64::try_from(len)
        .ok()
        .and_then(Term::try_small_int)
        .ok_or_else(badarg)
}

pub fn bif_reverse(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input] = args else {
        return Err(badarg());
    };
    let text = utf8_str(*input)?;
    let reversed: String = text.graphemes(true).rev().collect();
    context.alloc_binary(reversed.as_bytes())
}

pub fn bif_lowercase(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input] = args else {
        return Err(badarg());
    };
    let lowered = utf8_str(*input)?.to_lowercase();
    context.alloc_binary(lowered.as_bytes())
}

pub fn bif_uppercase(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input] = args else {
        return Err(badarg());
    };
    let raised = utf8_str(*input)?.to_uppercase();
    context.alloc_binary(raised.as_bytes())
}

pub fn bif_trim(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input, direction] = args else {
        return Err(badarg());
    };
    let text = utf8_str(*input)?;
    let direction = atom_name(*direction, context)?;
    let trimmed = match direction {
        "leading" => text.trim_start(),
        "trailing" => text.trim_end(),
        "both" => text.trim(),
        _ => return Err(badarg()),
    };
    // Own the bytes: alloc_binary may collect and move an inline source.
    let trimmed = trimmed.as_bytes().to_vec();
    context.alloc_binary(&trimmed)
}

pub fn bif_split(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input, pattern, option] = args else {
        return Err(badarg());
    };
    // Own the input up front: the per-part allocation loop below may collect,
    // so every part must borrow this owned buffer, never the process heap.
    let input = binary_bytes(*input)?.to_vec();
    let input = input.as_slice();
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
        terms.push(context.alloc_binary(part)?);
    }
    context.alloc_list(&terms)
}

pub fn bif_find(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input, pattern] = args else {
        return Err(badarg());
    };
    let input = binary_bytes(*input)?;
    let pattern = binary_bytes(*pattern)?;
    if let Some(index) = find_bytes(input, pattern) {
        // Own the bytes: alloc_binary may collect and move an inline source.
        let tail = input[index..].to_vec();
        context.alloc_binary(&tail)
    } else {
        atom_term("nomatch", context)
    }
}

/// `string:next_grapheme/1` over a UTF-8 binary.
///
/// Matches OTP's contract: `[]` for the empty string, otherwise an improper
/// cons `[Grapheme | RestBinary]` whose head is the codepoint integer for a
/// single-codepoint grapheme or a list of codepoint integers for a
/// multi-codepoint cluster.
pub fn bif_next_grapheme(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input] = args else {
        return Err(badarg());
    };
    let text = utf8_str(*input)?;
    let Some(grapheme) = text.graphemes(true).next() else {
        return Ok(Term::NIL);
    };
    let rest_bytes = text.as_bytes()[grapheme.len()..].to_vec();
    let codepoints: Vec<Term> = grapheme
        .chars()
        .map(|ch| Term::try_small_int(i64::from(ch as u32)).ok_or_else(badarg))
        .collect::<Result<_, _>>()?;
    context.with_rooted(&[], |context, roots| {
        let rest = context.alloc_binary(&rest_bytes)?;
        context.rooted_push(roots, rest)?;
        let head = if codepoints.len() == 1 {
            codepoints[0]
        } else {
            let head = context.alloc_list(&codepoints)?;
            context.rooted_push(roots, head)?;
            context.rooted(roots, 1)?
        };
        let rest = context.rooted(roots, 0)?;
        context.alloc_cons(head, rest)
    })
}

pub fn bif_pad(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input, length, direction, pad] = args else {
        return Err(badarg());
    };
    let text = utf8_str(*input)?;
    let target_len = non_negative_usize(*length)?;
    let direction = atom_name(*direction, context)?;
    let pad = binary_bytes(*pad)?;
    if pad.is_empty() {
        return Err(badarg());
    }
    let current_len = text.graphemes(true).count();
    let input = text.as_bytes();
    if current_len >= target_len {
        // Own the bytes: alloc_binary may collect and move an inline source.
        let owned = input.to_vec();
        return context.alloc_binary(&owned);
    }

    let needed = target_len - current_len;
    let (leading, trailing) = match direction {
        "leading" => (needed, 0),
        "trailing" => (0, needed),
        "both" => (needed / 2, needed - (needed / 2)),
        _ => return Err(badarg()),
    };
    let mut out = Vec::with_capacity(input.len() + needed * pad.len());
    append_pad(&mut out, pad, leading);
    out.extend_from_slice(input);
    append_pad(&mut out, pad, trailing);
    context.alloc_binary(&out)
}

pub fn bif_replace(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input, pattern, replacement, where_atom] = args else {
        return Err(badarg());
    };
    let input = binary_bytes(*input)?;
    let pattern = binary_bytes(*pattern)?;
    let replacement = binary_bytes(*replacement)?;
    if pattern.is_empty() {
        return Err(badarg());
    }
    let where_name = atom_name(*where_atom, context)?;
    let out = match where_name {
        "all" => replace_all(input, pattern, replacement),
        "leading" => replace_once(input, pattern, replacement, false),
        "trailing" => replace_once(input, pattern, replacement, true),
        _ => return Err(badarg()),
    };
    context.alloc_binary(&out)
}

/// `string:slice/3` indexed and measured in grapheme clusters.
///
/// Out-of-range start positions and lengths clamp to the available string —
/// OTP never raises for them.
pub fn bif_slice(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input, offset, length] = args else {
        return Err(badarg());
    };
    let text = utf8_str(*input)?;
    let offset = non_negative_usize(*offset)?;
    let length = non_negative_usize(*length)?;
    if length == 0 {
        return context.alloc_binary(&[]);
    }
    let mut indices = text.grapheme_indices(true).skip(offset);
    let Some((start, _)) = indices.next() else {
        return context.alloc_binary(&[]);
    };
    let end = indices.nth(length - 1).map_or(text.len(), |(end, _)| end);
    // Own the bytes: alloc_binary may collect and move an inline source.
    let sliced = text.as_bytes()[start..end].to_vec();
    context.alloc_binary(&sliced)
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
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn rfind_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(haystack.len());
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

fn replace_all(input: &[u8], pattern: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut index = 0;
    while let Some(relative) = find_bytes(&input[index..], pattern) {
        let match_start = index + relative;
        out.extend_from_slice(&input[index..match_start]);
        out.extend_from_slice(replacement);
        index = match_start + pattern.len();
    }
    out.extend_from_slice(&input[index..]);
    out
}

fn replace_once(input: &[u8], pattern: &[u8], replacement: &[u8], trailing: bool) -> Vec<u8> {
    let found = if trailing {
        rfind_bytes(input, pattern)
    } else {
        find_bytes(input, pattern)
    };
    if let Some(index) = found {
        let mut out = Vec::with_capacity(input.len() - pattern.len() + replacement.len());
        out.extend_from_slice(&input[..index]);
        out.extend_from_slice(replacement);
        out.extend_from_slice(&input[index + pattern.len()..]);
        out
    } else {
        input.to_vec()
    }
}

fn append_pad(out: &mut Vec<u8>, pad: &[u8], count: usize) {
    for index in 0..count {
        out.push(pad[index % pad.len()]);
    }
}

fn non_negative_usize(term: Term) -> Result<usize, Term> {
    term.as_small_int()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(badarg)
}

fn atom_term(name: &str, context: &mut ProcessContext) -> Result<Term, Term> {
    let table = context.atom_table().ok_or_else(badarg)?;
    Ok(Term::atom(table.intern(name)))
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

fn utf8_str(term: Term) -> Result<&'static str, Term> {
    std::str::from_utf8(binary_bytes(term)?).map_err(|_| badarg())
}

fn binary_bytes(term: Term) -> Result<&'static [u8], Term> {
    BinaryRef::new(term)
        .map(|binary| binary.as_bytes())
        .ok_or_else(badarg)
}

fn bool_term(value: bool) -> Term {
    Term::atom(if value { Atom::TRUE } else { Atom::FALSE })
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod ar1_row4_site14_tests {
    // ⛔ DEFECT-ASSERTING TESTS — READ THIS BEFORE TRUSTING A GREEN.
    //
    // These pin the MEASURED CORRUPT SURFACE of AR-1 row 4 at f993280. They do
    // NOT assert correct behaviour, so a green here means "the defect is still
    // present, exactly as measured" — never "this site is safe".
    //
    // ⇒ THEY GO RED WHEN AR-1 IS FIXED, AND THAT IS THE POINT. The fix lane
    // INVERTS them to assert correctness rather than deleting them; the pinned
    // counts below are the surface the fix has to move.

    use std::sync::Arc;

    use super::bif_split;
    use crate::atom::AtomTable;
    use crate::native::ProcessContext;
    use crate::process::Process;
    use crate::term::Term;
    use crate::term::binary::Binary;
    use crate::term::boxed::Cons;

    const WIDTH: usize = 12;

    fn part_of(index: usize) -> String {
        format!("p{index:0WIDTH$}")
    }

    /// Split an input of `parts` heap-sized parts on a heap of exactly `heap`
    /// words, and read the result back BY CONTENTS. Parts are 13 bytes so every
    /// one is heap-allocated — never immediates, or the carrier would never be
    /// live across an allocation and the probe could not fail.
    fn split_round_trip(parts: usize, heap: usize) -> Result<(), String> {
        let table = Arc::new(AtomTable::with_common_atoms());
        let all = table.intern("all");

        let mut process = Process::new(7, heap);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&table)));
        context.attach_process(&mut process, 0);

        let joined = (0..parts).map(part_of).collect::<Vec<_>>().join("|");
        let input = context
            .alloc_binary(joined.as_bytes())
            .map_err(|_| "input binary".to_string())?;
        let pattern = context
            .alloc_binary(b"|")
            .map_err(|_| "pattern binary".to_string())?;

        let list = bif_split(&[input, pattern, Term::atom(all)], &mut context)
            .map_err(|_| "bif_split returned an error term".to_string())?;

        let mut seen = 0usize;
        let mut tail = list;
        // HARD CAP: a stale carrier can make the list cyclic, and a reader that
        // spins forever reports nothing at all.
        let cap = parts * 2 + 16;
        while !tail.is_nil() {
            if seen > cap {
                return Err(format!(
                    "list did not terminate within {cap} cells — cyclic tail"
                ));
            }
            let cons = Cons::new(tail)
                .ok_or_else(|| format!("part {seen}: tail is not a cons — carrier went stale"))?;
            let binary = Binary::new(cons.head())
                .ok_or_else(|| format!("part {seen}: head is not a binary — carrier went stale"))?;
            let want = part_of(seen);
            if binary.as_bytes() != want.as_bytes() {
                return Err(format!(
                    "part {seen}: contents {:?} != {want:?}",
                    String::from_utf8_lossy(binary.as_bytes())
                ));
            }
            seen += 1;
            tail = cons.tail();
        }
        if seen != parts {
            return Err(format!("recovered {seen} parts, put {parts}"));
        }
        Ok(())
    }

    /// AR-1 row 4, site 14. Two-armed in both directions: hold the heap and grow
    /// the input, then hold the input and grow the heap. A failure that survives
    /// a roomy heap would be an allocator limit and would prove nothing.
    #[test]
    fn ar1_site14_bif_split_two_armed() {
        // MEASURED, and THE BAND IS ONE CELL WIDE. At heap 1024: 250 parts is
        // clean, 300 parts CORRUPTS, 350 parts is REFUSED by the terminal
        // `alloc_list`. The refusal MASKS the defect rather than disproving it —
        // instrumenting the production loop showed the collection still fires
        // (at part 254, available 2 -> 1020); the loop then finishes with 440
        // words and `alloc_list` needs 800 for the spine, so it refuses before
        // the corrupted accumulator can be read back.
        const HEAP: usize = 1024;
        const BIG: usize = 300;

        let control = split_round_trip(250, HEAP);
        assert!(
            control.is_ok(),
            "control arm: 250 parts on a {HEAP}-word heap must round-trip, got {control:?}. \
             A failure here would be an allocator limit and the red arm would prove nothing."
        );

        let red = split_round_trip(BIG, HEAP);
        // ⛔ THE RED ARM MUST EXCLUDE THE REFUSAL CLASS EXPLICITLY. `is_err()`
        // alone is satisfied by "bif_split returned an error term" — the
        // allocator refusing, which is precisely the ambiguity this arm exists
        // to rule out. An arm that accepts it passes for the wrong reason, and
        // this one did until the refusal was investigated instead of excluded.
        let corrupted = matches!(&red, Err(reason) if !reason.contains("returned an error term"));
        assert!(
            corrupted,
            "site 14 red-at-parent: {BIG} parts on a {HEAP}-word heap must CORRUPT the \
             accumulator (not merely be refused), got {red:?}"
        );

        let roomy = split_round_trip(BIG, 1536);
        assert!(
            roomy.is_ok(),
            "site 14 second direction: {BIG} parts on a 1536-word heap must be clean, got {roomy:?}"
        );

        let reason = red.unwrap_err();
        println!("site 14 RED: {reason}");
        eprintln!("site 14 RED: {reason}");
    }

    /// The surface the verdict is read off. Emitted per cell and to both streams.
    #[test]
    fn ar1_site14_sweep_surface() {
        // FINE-GRAINED DELIBERATELY. The first sweep here stepped 5/25/100/400
        // over heaps 256..65536 and found NO corruption cell at all — a
        // clean-then-refused surface that reads as DEFENDED. The corruption
        // band is one cell wide and sat between the steps.
        for heap in [1024usize, 1536, 2048] {
            for parts in [150usize, 200, 250, 300, 350] {
                let verdict = match split_round_trip(parts, heap) {
                    Ok(()) => "ok".to_string(),
                    Err(reason) => reason,
                };
                let line = format!("heap {heap:>6} x parts {parts:>4} : {verdict}");
                println!("{line}");
                eprintln!("{line}");
            }
        }
    }
}
