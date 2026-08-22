//! `uri_string` module natives.
//!
//! These serve the real `gleam_stdlib.erl` bytecode (`uri_parse`,
//! `parse_query`), so their contracts must match Erlang/OTP exactly:
//! `parse/1` returns a map containing only the components present in the
//! input (with an integer `port`), and `dissect_query/1` returns a list of
//! `{Key, Value}` pairs with `true` for valueless keys, decoding
//! `application/x-www-form-urlencoded` escapes.

use crate::atom::Atom;
use crate::native::ProcessContext;
use crate::term::Term;
use crate::term::binary_ref::BinaryRef;
use crate::term::heap_borrow::HeapBorrow;

/// `uri_string:parse/1` over a UTF-8 binary, RFC 3986 component split.
pub fn bif_uri_string_parse(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [input] = args else {
        return Err(badarg());
    };
    let owned = binary_text(*input, context.borrow_terms())?;
    let text = owned.as_str();

    let (rest, fragment) = match text.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (text, None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((rest, query)) => (rest, Some(query)),
        None => (rest, None),
    };
    let (scheme, rest) = split_scheme(rest);
    let (authority, path) = split_authority(rest);
    let mut userinfo = None;
    let mut host = None;
    let mut port = None;
    if let Some(authority) = authority {
        let (user_part, host_part) = match authority.split_once('@') {
            Some((user, host)) => (Some(user), host),
            None => (None, authority),
        };
        userinfo = user_part;
        let (host_text, port_text) = split_host_port(host_part);
        host = Some(host_text);
        if let Some(port_text) = port_text {
            // OTP keeps an empty port as `port => undefined` and rejects a
            // non-numeric one outright.
            if port_text.is_empty() {
                port = Some(PortComponent::Undefined);
            } else if port_text.chars().all(|ch| ch.is_ascii_digit()) {
                match port_text.parse::<u16>() {
                    Ok(value) => port = Some(PortComponent::Number(value)),
                    Err(_) => return error_tuple(context, "invalid_uri", ":"),
                }
            } else {
                return error_tuple(context, "invalid_uri", ":");
            }
        }
    }

    // AR-1 site 7. The carrier used to be a bare `Vec<Term>` of `values`
    // holding boxed binaries across further `alloc_binary` calls, any of which
    // can collect. Keys and values now accumulate as ONE alternating run in the
    // process root stack and reach `alloc_map` through
    // `TermAccumulator::to_map_pairs`, which splits the run and roots both
    // halves. The keys are atoms and were never at risk; interleaving them
    // keeps a single carrier rather than two that must stay the same length.
    context.with_accumulator(|context, entries| {
        if let Some(scheme) = scheme {
            let key = atom(context, "scheme")?;
            entries.push(context, key)?;
            let value = context.alloc_binary(scheme.as_bytes())?;
            entries.push(context, value)?;
        }
        if let Some(userinfo) = userinfo {
            let key = atom(context, "userinfo")?;
            entries.push(context, key)?;
            let value = context.alloc_binary(userinfo.as_bytes())?;
            entries.push(context, value)?;
        }
        if let Some(host) = host {
            let key = atom(context, "host")?;
            entries.push(context, key)?;
            let value = context.alloc_binary(host.as_bytes())?;
            entries.push(context, value)?;
        }
        if let Some(port) = port {
            let key = atom(context, "port")?;
            entries.push(context, key)?;
            let value = match port {
                PortComponent::Number(value) => {
                    Term::try_small_int(i64::from(value)).ok_or_else(badarg)?
                }
                PortComponent::Undefined => atom(context, "undefined")?,
            };
            entries.push(context, value)?;
        }
        let key = atom(context, "path")?;
        entries.push(context, key)?;
        let value = context.alloc_binary(path.as_bytes())?;
        entries.push(context, value)?;
        if let Some(query) = query {
            let key = atom(context, "query")?;
            entries.push(context, key)?;
            let value = context.alloc_binary(query.as_bytes())?;
            entries.push(context, value)?;
        }
        if let Some(fragment) = fragment {
            let key = atom(context, "fragment")?;
            entries.push(context, key)?;
            let value = context.alloc_binary(fragment.as_bytes())?;
            entries.push(context, value)?;
        }
        entries.to_map_pairs(context)
    })
}

/// `uri_string:dissect_query/1` decoding `application/x-www-form-urlencoded`.
pub fn bif_uri_string_dissect_query(
    args: &[Term],
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    let [input] = args else {
        return Err(badarg());
    };
    let text = binary_text(*input, context.borrow_terms())?;
    if text.is_empty() {
        return Ok(Term::NIL);
    }

    let mut pairs: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    for part in text.split('&') {
        match part.split_once('=') {
            Some((key, value)) => {
                let (Some(key), Some(value)) = (form_decode(key), form_decode(value)) else {
                    return error_tuple(context, "invalid_query", part);
                };
                pairs.push((key, Some(value)));
            }
            None => {
                let Some(key) = form_decode(part) else {
                    return error_tuple(context, "invalid_query", part);
                };
                pairs.push((key, None));
            }
        }
    }

    // AR-1 sites 8 and 9, fixed together because they are the same loop.
    //
    // Site 8 was the bare `Vec<Term>` of pair tuples, held across further
    // `alloc_binary` and `alloc_tuple` calls — that becomes the accumulator.
    //
    // Site 9 was `key`: ONE boxed binary held across the value's
    // `alloc_binary`. It takes `with_rooted` DIRECTLY rather than the
    // accumulator, because it is a single term with a scope-shaped lifetime
    // and not a run — the accumulator would be the wrong tool and would leave
    // the key in the list slot it does not belong in.
    context.with_accumulator(|context, terms| {
        for (key, value) in pairs {
            let tuple = context.with_rooted(&[], |context, roots| {
                let key = context.alloc_binary(&key)?;
                context.rooted_push(roots, key)?;
                let value = match value {
                    Some(bytes) => context.alloc_binary(&bytes)?,
                    None => Term::atom(Atom::TRUE),
                };
                // Re-read the key AFTER the value allocation: a collection
                // there forwards it, and the pre-fix bug was using the stale
                // copy from before.
                let key = context.rooted(roots, 0)?;
                context.alloc_tuple(&[key, value])
            })?;
            terms.push(context, tuple)?;
        }
        terms.to_list(context)
    })
}

/// `maps:get/2` raising `{badkey, Key}` for missing keys, matching the BIF.
pub fn bif_maps_get_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [key, map_term] = args else {
        return Err(badarg());
    };
    let map = crate::term::boxed::Map::new(*map_term).ok_or_else(badarg)?;
    match map.get(*key) {
        Some(value) => Ok(value),
        None => {
            let badkey = atom(context, "badkey")?;
            Err(context.alloc_tuple(&[badkey, *key])?)
        }
    }
}

/// `maps:get/3` returning the default for missing keys.
pub fn bif_maps_get_3(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [key, map_term, default] = args else {
        return Err(badarg());
    };
    let map = crate::term::boxed::Map::new(*map_term).ok_or_else(badarg)?;
    Ok(map.get(*key).unwrap_or(*default))
}

/// A parsed authority port: numeric, or present-but-empty (`undefined`).
enum PortComponent {
    Number(u16),
    Undefined,
}

/// Splits a leading `scheme:` when the scheme is RFC 3986-valid.
fn split_scheme(text: &str) -> (Option<&str>, &str) {
    let Some(colon) = text.find(':') else {
        return (None, text);
    };
    let candidate = &text[..colon];
    if candidate.is_empty() {
        return (None, text);
    }
    let mut chars = candidate.chars();
    let valid_first = chars.next().is_some_and(|ch| ch.is_ascii_alphabetic());
    let valid_rest =
        chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '-' || ch == '.');
    // A '/' or '?' before the colon means it belongs to the path, not a scheme.
    if valid_first && valid_rest && !candidate.contains('/') {
        (Some(candidate), &text[colon + 1..])
    } else {
        (None, text)
    }
}

/// Splits `//authority` from the path remainder.
fn split_authority(text: &str) -> (Option<&str>, &str) {
    let Some(after) = text.strip_prefix("//") else {
        return (None, text);
    };
    match after.find('/') {
        Some(slash) => (Some(&after[..slash]), &after[slash..]),
        None => (Some(after), ""),
    }
}

/// Splits `host[:port]`, honouring IPv6 bracket notation.
fn split_host_port(text: &str) -> (&str, Option<&str>) {
    if let Some(rest) = text.strip_prefix('[')
        && let Some(close) = rest.find(']')
    {
        let host = &rest[..close];
        let remainder = &rest[close + 1..];
        return match remainder.strip_prefix(':') {
            Some(port) => (host, Some(port)),
            None => (host, None),
        };
    }
    match text.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (text, None),
    }
}

/// Decodes a form-urlencoded component (`+` as space, `%XX` escapes).
fn form_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' => {
                let high = hex_value(*bytes.get(index + 1)?)?;
                let low = hex_value(*bytes.get(index + 2)?)?;
                out.push(high * 16 + low);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn error_tuple(context: &mut ProcessContext, reason: &str, detail: &str) -> Result<Term, Term> {
    let error = Term::atom(Atom::ERROR);
    let reason = atom(context, reason)?;
    // Own the bytes: alloc_binary may collect, and a caller's detail must
    // never be read from a moved heap source afterwards.
    let detail_bytes = detail.as_bytes().to_vec();
    let detail = context.alloc_binary(&detail_bytes)?;
    context.alloc_tuple(&[error, reason, detail])
}

/// Owns the input text up front: every component slice the callers derive
/// borrows this owned copy, never the process heap — the sequential
/// `alloc_binary` calls below may collect and move an inline source.
fn binary_text(term: Term, heap: HeapBorrow<'_>) -> Result<String, Term> {
    let binary = BinaryRef::new(term).ok_or_else(badarg)?;
    std::str::from_utf8(binary.as_bytes(heap))
        .map(str::to_owned)
        .map_err(|_| badarg())
}

fn atom(context: &mut ProcessContext, name: &str) -> Result<Term, Term> {
    let table = context.atom_table().ok_or_else(badarg)?;
    Ok(Term::atom(table.intern(name)))
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod ar1_row4_site7_tests {
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

    use super::{atom, bif_uri_string_parse};
    use crate::atom::AtomTable;
    use crate::native::ProcessContext;
    use crate::process::Process;
    use crate::term::Term;
    use crate::term::binary::Binary;
    use crate::term::boxed::Map;

    /// Components sized to stay INLINE (<= 64 bytes), so each one occupies
    /// nursery words instead of collapsing to a flat 3-word ProcBin.
    const COMPONENT: usize = 60;

    fn component(tag: char) -> String {
        std::iter::repeat_n(tag, COMPONENT).collect()
    }

    /// Which body the cell drives. ⛔ The replica exists because inverting this
    /// probe killed its own positive control: a non-empty corruption set used to
    /// prove the sweep applied real pressure, and post-fix nothing at the
    /// production site can.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Arm {
        Fixed,
        UnrootedReplica,
    }

    /// ⛔⛔ THE SYNTHETIC POSITIVE — `bif_uri_string_parse`'s ACCUMULATION
    /// EXACTLY AS IT WAS BEFORE THE FIX, and it must stay that way.
    ///
    /// Two parallel bare `Vec<Term>`s, the `values` one holding boxed binaries
    /// across further `alloc_binary` calls, terminating in
    /// `alloc_map(&keys, &values)` — which roots both AFTER they have gone
    /// stale. Only the accumulation is reproduced, not the URI parsing: the
    /// parse allocates nothing and cannot witness this defect.
    /// ⛔ Do NOT migrate it onto the accumulator.
    fn parse_map_unrooted_replica(
        context: &mut ProcessContext,
        components: &[(&str, &[u8])],
    ) -> Result<Term, Term> {
        let mut keys = Vec::new();
        let mut values = Vec::new();
        for (name, bytes) in components {
            keys.push(atom(context, name)?);
            values.push(context.alloc_binary(bytes)?);
        }
        context.alloc_map(&keys, &values)
    }

    /// Parse a URI with all seven components present on a heap of exactly
    /// `heap` words, and read the result map back BY CONTENTS.
    fn parse_round_trip(heap: usize, margin: usize, arm: Arm) -> Result<(), String> {
        let table = Arc::new(AtomTable::with_common_atoms());
        let mut process = Process::new(9, heap);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&table)));
        context.attach_process(&mut process, 0);

        let user = component('u');
        let host = component('h');
        let path = component('p');
        let query = component('q');
        let fragment = component('f');
        let uri = format!("https://{user}@{host}:8080/{path}?{query}#{fragment}");

        let input = context
            .alloc_binary(uri.as_bytes())
            .map_err(|_| "input binary".to_string())?;

        // PRE-FILL to a measured margin. Without this the parse consumes only
        // ~13 words against 61 available even at the smallest heap, so no
        // collection can occur and a clean result reports the ABSENCE OF
        // PRESSURE, not the presence of safety. The filler terms are held in a
        // plain Vec and are therefore UNROOTED — exactly like the production
        // carrier — so a collection frees them and the parse proceeds.
        let mut filler = Vec::new();
        loop {
            let available = context.process_heap().map(|h| h.available()).unwrap_or(0);
            if available <= margin {
                break;
            }
            match context.alloc_binary(&[0xAB; 32]) {
                Ok(term) => filler.push(term),
                Err(_) => break,
            }
        }

        let term = match arm {
            Arm::Fixed => bif_uri_string_parse(&[input], &mut context)
                .map_err(|_| "bif_uri_string_parse returned an error term".to_string())?,
            Arm::UnrootedReplica => {
                // The same seven components the parse would produce, in the
                // same order, so the allocation sequence matches.
                let path_value = format!("/{path}");
                let components: Vec<(&str, &[u8])> = vec![
                    ("scheme", b"https".as_slice()),
                    ("userinfo", user.as_bytes()),
                    ("host", host.as_bytes()),
                    ("path", path_value.as_bytes()),
                    ("query", query.as_bytes()),
                    ("fragment", fragment.as_bytes()),
                ];
                parse_map_unrooted_replica(&mut context, &components)
                    .map_err(|_| "replica returned an error term".to_string())?
            }
        };

        let map =
            Map::new(term).ok_or_else(|| "result is not a map — carrier went stale".to_string())?;

        // Expected VALUE contents keyed by the atom name of the key. `port` is a
        // small int (an immediate) and is excluded — an immediate needs no
        // allocation, so it cannot witness this defect.
        let expected: Vec<(&str, String)> = vec![
            ("scheme", "https".to_string()),
            ("userinfo", user),
            ("host", host),
            ("path", format!("/{path}")),
            ("query", query),
            ("fragment", fragment),
        ];

        for index in 0..map.len() {
            let key = map
                .key(index)
                .ok_or_else(|| format!("entry {index}: key slot absent"))?;
            let atom = key
                .as_atom()
                .ok_or_else(|| format!("entry {index}: key is not an atom — carrier went stale"))?;
            let name = table
                .resolve(atom)
                .ok_or_else(|| format!("entry {index}: key atom does not resolve"))?
                .to_string();

            let value = map
                .value(index)
                .ok_or_else(|| format!("entry {index}: value slot absent"))?;

            if name == "port" {
                continue;
            }
            let Some((_, want)) = expected.iter().find(|(key_name, _)| *key_name == name) else {
                return Err(format!("entry {index}: unexpected key {name:?}"));
            };
            let binary = Binary::new(value).ok_or_else(|| {
                format!(
                    "entry {index} ({name}): value is not a binary — carrier `values` went stale"
                )
            })?;
            if binary.as_bytes(context.borrow_terms()) != want.as_bytes() {
                return Err(format!(
                    "entry {index} ({name}): contents {:?} != expected",
                    String::from_utf8_lossy(binary.as_bytes(context.borrow_terms()))
                ));
            }
        }
        Ok(())
    }

    /// AR-1 row 4, site 7. Only ONE knob is available here, so the two arms are
    /// two heap sizes against an identical input: the small arm must corrupt and
    /// the large arm must be clean. If both failed it would be an allocator
    /// limit and neither would prove anything.
    #[test]
    fn ar1_site7_uri_parse_two_armed() {
        // ⛔⛔ POSITIVE CONTROL FIRST, and it licenses everything below it.
        let (control_corrupt, control_clean) = sweep(Arm::UnrootedReplica);
        assert!(
            control_corrupt > 0,
            "POSITIVE CONTROL DEAD: the unrooted replica no longer corrupts anywhere in the sweep \
             ({control_corrupt} corrupt / {control_clean} clean). The pressure regime is gone, so \
             the fixed arm's zeros below mean nothing. ⛔ A refusal does not count as corruption."
        );
        assert!(
            control_clean > 0,
            "NEGATIVE CONTROL DEAD: no cell was clean under the replica, which indicts the reader \
             or the pre-fill rather than the carrier."
        );

        // ✅ THE CLAIM.
        let (fixed_corrupt, fixed_clean) = sweep(Arm::Fixed);
        assert_eq!(
            fixed_corrupt, 0,
            "site 7 is NOT rooted: {fixed_corrupt} cells still lost the `values` carrier while \
             the replica corrupted {control_corrupt} in the same run"
        );
        assert!(
            fixed_clean > 0,
            "site 7: the fixed arm produced no clean cell at all, so the zero above measures \
             refusals rather than safety"
        );
    }

    /// One arm's full sweep, returning `(corrupt, clean)`.
    fn sweep(arm: Arm) -> (usize, usize) {
        let mut band = Vec::new();
        for (heap, margin) in [
            (256usize, 4usize),
            (256, 8),
            (256, 16),
            (256, 24),
            (256, 32),
            (256, 48),
            (1024, 4),
            (1024, 8),
            (1024, 16),
            (1024, 24),
            (1024, 32),
            (1024, 48),
            (1024, 64),
            (1024, 96),
            (4096, 16),
            (4096, 32),
            (4096, 64),
            (4096, 128),
        ] {
            let verdict = match parse_round_trip(heap, margin, arm) {
                Ok(()) => "ok".to_string(),
                Err(reason) => reason,
            };
            let line = format!("site 7 [{arm:?}] heap {heap:>5} margin {margin:>4} : {verdict}");
            println!("{line}");
            eprintln!("{line}");
            band.push((heap, verdict));
        }

        // A corruption cell is one that is neither clean nor a refusal — the
        // refusal class is excluded EXPLICITLY, since an error at the conversion
        // could be the allocator declining rather than this defect.
        let corrupted: Vec<_> = band
            .iter()
            .filter(|(_, verdict)| verdict != "ok" && !verdict.contains("returned an error term"))
            .collect();
        let clean: Vec<_> = band.iter().filter(|(_, verdict)| verdict == "ok").collect();

        println!(
            "site 7 [{arm:?}]: {} corruption cells, {} clean cells",
            corrupted.len(),
            clean.len()
        );
        eprintln!(
            "site 7 [{arm:?}]: {} corruption cells, {} clean cells",
            corrupted.len(),
            clean.len()
        );
        for (heap, verdict) in &corrupted {
            println!("site 7 [{arm:?}] RED at heap {heap}: {verdict}");
            eprintln!("site 7 [{arm:?}] RED at heap {heap}: {verdict}");
        }

        (corrupted.len(), clean.len())
    }
}

#[cfg(test)]
mod ar1_row4_tests {
    // ⛔ DEFECT-ASSERTING TESTS — READ THIS BEFORE TRUSTING A GREEN.
    //
    // These pin the MEASURED CORRUPT SURFACE of AR-1 row 4 at f993280. They do
    // NOT assert correct behaviour, so a green here means "the defect is still
    // present, exactly as measured" — never "this site is safe".
    //
    // ⇒ THEY GO RED WHEN AR-1 IS FIXED, AND THAT IS THE POINT. The fix lane
    // INVERTS them to assert correctness rather than deleting them; the pinned
    // counts below are the surface the fix has to move.

    use crate::atom::Atom;
    use crate::native::ProcessContext;
    use crate::process::Process;
    use crate::term::Term;
    use crate::term::binary::Binary;
    use crate::term::boxed::{Cons, Tuple};

    use super::bif_uri_string_dissect_query;

    /// Which body the cell drives.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Arm {
        Fixed,
        UnrootedReplica,
    }

    /// ⛔⛔ THE SYNTHETIC POSITIVE — `bif_uri_string_dissect_query`'s
    /// accumulation EXACTLY AS IT WAS BEFORE THE FIX, and it must stay that way.
    ///
    /// It carries BOTH defects at once, which is why sites 8 and 9 share it: the
    /// bare `Vec<Term>` of tuples (site 8) and the single `key` held across the
    /// value's `alloc_binary` (site 9). Only the accumulation is reproduced —
    /// the form-decoding allocates nothing and cannot witness either defect.
    /// ⛔ Do NOT migrate it onto the accumulator or onto `with_rooted`.
    fn dissect_unrooted_replica(
        context: &mut ProcessContext,
        pairs: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    ) -> Result<Term, Term> {
        let mut terms = Vec::with_capacity(pairs.len());
        for (key, value) in pairs {
            let key = context.alloc_binary(&key)?;
            let value = match value {
                Some(bytes) => context.alloc_binary(&bytes)?,
                None => Term::atom(Atom::TRUE),
            };
            terms.push(context.alloc_tuple(&[key, value])?);
        }
        context.alloc_list(&terms)
    }

    fn context(process: &mut Process) -> ProcessContext<'_> {
        let mut context = ProcessContext::new();
        context.attach_process(process, 0);
        context
    }

    /// Drive `uri_string:dissect_query/1` with `pairs` key=value pairs whose
    /// key and value are both long enough to be heap-allocated, and read the
    /// result back BY CONTENTS. Returns Err with a reason when the result is
    /// not the list that was put in.
    fn dissect(pairs: usize, heap: usize, arm: Arm) -> Result<(), String> {
        const WIDTH: usize = 12;

        let mut process = Process::new(1, heap);
        let mut context = context(&mut process);

        let query: String = (0..pairs)
            .map(|i| format!("k{i:0WIDTH$}=v{i:0WIDTH$}"))
            .collect::<Vec<_>>()
            .join("&");
        let input = context
            .alloc_binary(query.as_bytes())
            .map_err(|_| "input binary".to_string())?;

        let list = match arm {
            Arm::Fixed => bif_uri_string_dissect_query(&[input], &mut context)
                .map_err(|_| "dissect_query returned an error term".to_string())?,
            Arm::UnrootedReplica => {
                // The same decoded pairs the BIF would hand its accumulation
                // loop, so the allocation sequence matches.
                let decoded: Vec<(Vec<u8>, Option<Vec<u8>>)> = (0..pairs)
                    .map(|i| {
                        (
                            format!("k{i:0WIDTH$}").into_bytes(),
                            Some(format!("v{i:0WIDTH$}").into_bytes()),
                        )
                    })
                    .collect();
                dissect_unrooted_replica(&mut context, decoded)
                    .map_err(|_| "replica returned an error term".to_string())?
            }
        };

        let mut seen = 0usize;
        let mut tail = list;
        while !tail.is_nil() {
            let cons = Cons::new(tail).ok_or_else(|| {
                format!("element {seen}: result is not a proper list — the accumulator went stale")
            })?;
            let tuple = Tuple::new(cons.head()).ok_or_else(|| {
                format!("element {seen}: not a tuple — the accumulator went stale")
            })?;
            if tuple.arity() != 2 {
                return Err(format!("element {seen}: arity {} not 2", tuple.arity()));
            }
            let key = Binary::new(tuple.get(0).expect("key element")).ok_or_else(|| {
                format!("element {seen}: key is not a binary — carrier `key` went stale")
            })?;
            let value = Binary::new(tuple.get(1).expect("value element"))
                .ok_or_else(|| format!("element {seen}: value is not a binary"))?;
            let want_key = format!("k{seen:0WIDTH$}");
            let want_value = format!("v{seen:0WIDTH$}");
            if key.as_bytes(context.borrow_terms()) != want_key.as_bytes() {
                return Err(format!(
                    "element {seen}: key contents {:?} != {want_key:?}",
                    String::from_utf8_lossy(key.as_bytes(context.borrow_terms()))
                ));
            }
            if value.as_bytes(context.borrow_terms()) != want_value.as_bytes() {
                return Err(format!(
                    "element {seen}: value contents {:?} != {want_value:?}",
                    String::from_utf8_lossy(value.as_bytes(context.borrow_terms()))
                ));
            }
            seen += 1;
            tail = cons.tail();
        }
        if seen != pairs {
            return Err(format!("recovered {seen} pairs, put {pairs}"));
        }
        Ok(())
    }

    /// AR-1 row 4, sites 8 (`terms`) and 9 (`key`) — ✅ INVERTED.
    ///
    /// Still two-armed on INPUT SIZE per Amendment 6, and now two-armed on BODY
    /// as well: the size arms establish that the pressure is a collection rather
    /// than an allocator limit, and the body arms establish that the pressure is
    /// still there after the fix.
    #[test]
    fn ar1_sites_8_9_dissect_query_two_armed() {
        // The heap is held CONSTANT across every cell so the only variable is
        // whether the accumulation outruns it. 4096 words was measured: at 200
        // pairs the call completes without collecting, at 400 it must collect.
        const HEAP: usize = 4096;

        // ⛔⛔ POSITIVE CONTROL FIRST, and it licenses everything below it.
        // Small input: must succeed, or the pressure below is an allocator
        // limit and proves nothing.
        let control_small = dissect(200, HEAP, Arm::UnrootedReplica);
        assert!(
            control_small.is_ok(),
            "CONTROL ARM DEAD: 200 pairs on a {HEAP}-word heap must succeed under the replica, \
             got {control_small:?}. A failure here would be an allocator limit."
        );
        // Large input: must still corrupt.
        let control_red = dissect(400, HEAP, Arm::UnrootedReplica);
        assert!(
            control_red.is_err(),
            "POSITIVE CONTROL DEAD: the unrooted replica no longer corrupts at 400 pairs on a \
             {HEAP}-word heap (got {control_red:?}). The pressure regime is gone, so the fixed \
             arm's success below means nothing."
        );
        let reason = control_red.unwrap_err();
        assert!(
            !reason.contains("returned an error term"),
            "POSITIVE CONTROL IS A REFUSAL, NOT CORRUPTION: {reason}. A refusal is evidence of \
             nothing about rooting — it is the exact ambiguity this arm exists to rule out."
        );
        println!("sites 8/9 CONTROL still red: {reason}");
        eprintln!("sites 8/9 CONTROL still red: {reason}");

        // ✅ THE CLAIM. Same heap, same inputs, through the rooted body.
        let fixed_small = dissect(200, HEAP, Arm::Fixed);
        assert!(
            fixed_small.is_ok(),
            "sites 8/9: 200 pairs must round-trip, got {fixed_small:?}"
        );
        let fixed_large = dissect(400, HEAP, Arm::Fixed);
        assert!(
            fixed_large.is_ok(),
            "sites 8/9 are NOT rooted: 400 pairs on a {HEAP}-word heap still lost a carrier, got \
             {fixed_large:?}, while the replica corrupted in the same run"
        );
    }
}
