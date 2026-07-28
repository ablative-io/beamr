//! GC-pressure regression tests for native rooting.
//!
//! These tests force collections in the middle of BIF allocation sequences
//! with boxed (heap-pointer) terms live, which corrupted results before the
//! rooted-scope mechanism existed: x-registers above the BIF arity were not
//! GC roots, and accumulated `Vec<Term>` state was never traced.

use crate::atom::AtomTable;
use crate::native::{NativeContinuation, ProcessContext};
use crate::process::Process;
use crate::term::Term;
use crate::term::boxed::{Cons, Float, Tuple};

use super::lists_bifs::list_from_vec;
use super::lists_hof_bifs::ListsHofState;

fn context(process: &mut Process) -> ProcessContext<'_> {
    let mut context = ProcessContext::new();
    context.set_atom_table(Some(std::sync::Arc::new(AtomTable::with_common_atoms())));
    context.attach_process(process, 0);
    context
}

fn alloc_floats(ctx: &mut ProcessContext<'_>, count: usize) -> Vec<Term> {
    (0..count)
        .map(|index| {
            #[allow(clippy::cast_precision_loss)]
            ctx.alloc_float(index as f64 + 0.5).expect("float alloc")
        })
        .collect()
}

#[test]
fn list_from_vec_preserves_boxed_floats_under_gc_pressure() {
    // Small heap so the reserve inside list_from_vec must collect.
    let mut process = Process::new(1, 96);
    let mut ctx = context(&mut process);

    let floats = alloc_floats(&mut ctx, 24);
    let list = list_from_vec(&floats, &mut ctx).expect("list_from_vec");

    let mut current = list;
    let mut seen = 0usize;
    while !current.is_nil() {
        let cons = Cons::new(current).expect("cons cell");
        let float = Float::new(cons.head()).expect("element must still be a float");
        #[allow(clippy::cast_precision_loss)]
        let expected = seen as f64 + 0.5;
        assert!((float.value() - expected).abs() < f64::EPSILON);
        seen += 1;
        current = cons.tail();
    }
    assert_eq!(seen, 24);
}

#[test]
fn list_from_vec_handles_more_elements_than_x_registers() {
    // The previous implementation spilled elements into x-registers and
    // panicked past index 1023; the rooted scope has no such bound.
    let mut process = Process::new(1, 4096);
    let mut ctx = context(&mut process);

    let elements: Vec<Term> = (0..1500).map(Term::small_int).collect();
    let list = list_from_vec(&elements, &mut ctx).expect("large list");

    let mut current = list;
    let mut seen = 0i64;
    while !current.is_nil() {
        let cons = Cons::new(current).expect("cons cell");
        assert_eq!(cons.head().as_small_int(), Some(seen));
        seen += 1;
        current = cons.tail();
    }
    assert_eq!(seen, 1500);
}

#[test]
fn alloc_tuple_roots_boxed_arguments_across_gc() {
    let mut process = Process::new(1, 64);
    let mut ctx = context(&mut process);

    let a = ctx.alloc_float(1.25).expect("float");
    let b = ctx.alloc_float(2.5).expect("float");
    // Force pressure: this tuple alloc must reserve and may collect, moving
    // a and b. The allocator roots them internally.
    let tuple = ctx.alloc_tuple(&[a, b]).expect("tuple");

    let tuple = Tuple::new(tuple).expect("tuple term");
    let a = Float::new(tuple.get(0).expect("a")).expect("a is float");
    let b = Float::new(tuple.get(1).expect("b")).expect("b is float");
    assert!((a.value() - 1.25).abs() < f64::EPSILON);
    assert!((b.value() - 2.5).abs() < f64::EPSILON);
}

#[test]
fn rooted_push_accumulation_survives_gc() {
    let mut process = Process::new(1, 96);
    let mut ctx = context(&mut process);

    let result = ctx.with_rooted(&[], |ctx, roots| {
        for index in 0..16 {
            #[allow(clippy::cast_precision_loss)]
            let float = ctx.alloc_float(index as f64)?;
            ctx.rooted_push(roots, float)?;
        }
        (0..ctx.rooted_len(roots))
            .map(|index| ctx.rooted(roots, index))
            .collect::<Result<Vec<_>, _>>()
    });

    let values = result.expect("rooted accumulation");
    for (index, term) in values.iter().enumerate() {
        let float = Float::new(*term).expect("accumulated float survives");
        #[allow(clippy::cast_precision_loss)]
        let expected = index as f64;
        assert!((float.value() - expected).abs() < f64::EPSILON);
    }
}

#[test]
fn native_continuation_terms_are_gc_roots() {
    let mut process = Process::new(1, 96);

    // Build continuation state holding boxed floats, as a lists:map
    // trampoline does between closure calls.
    let (fun, remaining, results) = {
        let mut ctx = context(&mut process);
        let fun = ctx.alloc_float(9.75).expect("stand-in fun term");
        let remaining = alloc_floats(&mut ctx, 4);
        let results = alloc_floats(&mut ctx, 3);
        (fun, remaining, results)
    };
    process.push_native_continuation(
        NativeContinuation::Lists(ListsHofState::Map {
            fun,
            remaining: remaining.clone(),
            results: results.clone(),
        }),
        0,
    );

    // Force a full collection cycle with no live x registers.
    crate::gc::ensure_space(&mut process, 64, 0).expect("gc");

    let continuation = process
        .take_native_continuation()
        .expect("continuation survives");
    let NativeContinuation::Lists(ListsHofState::Map {
        fun,
        remaining,
        results,
    }) = continuation
    else {
        panic!("continuation shape preserved");
    };
    let fun = Float::new(fun).expect("fun term forwarded");
    assert!((fun.value() - 9.75).abs() < f64::EPSILON);
    assert_eq!(remaining.len(), 4);
    assert_eq!(results.len(), 3);
    for term in remaining.iter().chain(results.iter()) {
        let _ = Float::new(*term).expect("continuation element forwarded");
    }
}

// --- as_bytes borrow-across-alloc walls (fix lane, AION-ENCODE-GC-DEFECT) ---
// Each wall generalizes the audited probe: an inline (≤64 B) input binary is
// rooted in X0, the nursery is filled until the BIF's result allocation must
// collect, and the output is asserted byte-exact. Red at the audited main
// (silently zeroed output — the young reset zero-fills moved sources);
// green once the BIF owns its bytes before the allocating call.

use crate::term::binary_ref::BinaryRef;
use crate::term::shared_binary::alloc_binary_word_count;

fn inline_input(process: &mut Process, bytes: &[u8]) -> Term {
    let term = {
        let mut ctx = context(process);
        ctx.alloc_binary(bytes).expect("input binary")
    };
    process.set_x_reg(0, term);
    term
}

fn force_collect_geometry(process: &mut Process, result_len: usize) {
    let needed = alloc_binary_word_count(result_len);
    let mut ctx = context(process);
    while ctx.process_heap().expect("heap").available() >= needed {
        ctx.alloc_cons(Term::small_int(1), Term::NIL)
            .expect("filler");
    }
}

fn shared_atoms() -> std::sync::Arc<AtomTable> {
    std::sync::Arc::new(AtomTable::with_common_atoms())
}

// One atom table must be shared across every context in a wall: the BIF
// interns atoms (map keys, direction atoms) in its context's table, and the
// assertions must compare against the same interned indices.
fn live_context<'p>(
    process: &'p mut Process,
    live_x: u16,
    atoms: &std::sync::Arc<AtomTable>,
) -> ProcessContext<'p> {
    let mut context = ProcessContext::new();
    context.set_atom_table(Some(std::sync::Arc::clone(atoms)));
    context.attach_process(process, usize::from(live_x));
    context
}

fn result_bytes(term: Term) -> Vec<u8> {
    BinaryRef::new(term)
        .expect("binary result")
        .as_bytes()
        .to_vec()
}

#[test]
fn list_append_binary_arm_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let input = inline_input(&mut process, b"append-me \xE2\x80\x94 exact");
    force_collect_geometry(&mut process, 19);
    let mut ctx = live_context(&mut process, 1, &atoms);
    let result =
        super::super::gate3_bifs::bif_list_append(&[input, Term::NIL], &mut ctx).expect("++");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    assert_eq!(result_bytes(result), b"append-me \xE2\x80\x94 exact");
}

#[test]
fn binary_part_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let raw: Vec<u8> = (1..=40).collect();
    let input = inline_input(&mut process, &raw);
    force_collect_geometry(&mut process, 20);
    let mut ctx = live_context(&mut process, 1, &atoms);
    let result = super::misc_bifs::bif_binary_part(
        &[input, Term::small_int(10), Term::small_int(20)],
        &mut ctx,
    )
    .expect("binary_part");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    let expected: Vec<u8> = (11..=30).collect();
    assert_eq!(result_bytes(result), expected);
}

#[test]
fn string_trim_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let input = inline_input(&mut process, b"  \xE2\x80\x94 trim exact \xE2\x80\x94  ");
    let direction = Term::atom(atoms.intern("both"));
    force_collect_geometry(&mut process, 18);
    let mut ctx = live_context(&mut process, 1, &atoms);
    let result = super::string_bifs::bif_trim(&[input, direction], &mut ctx).expect("trim");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    assert_eq!(
        result_bytes(result),
        b"\xE2\x80\x94 trim exact \xE2\x80\x94"
    );
}

#[test]
fn string_split_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let input = inline_input(&mut process, b"aa\xE2\x80\x94bb\xE2\x80\x94cc");
    let pattern = {
        let mut ctx = live_context(&mut process, 1, &atoms);
        let p = ctx.alloc_binary(b"\xE2\x80\x94").expect("pattern");
        ctx.detach_process();
        p
    };
    process.set_x_reg(1, pattern);
    let all = Term::atom(atoms.intern("all"));
    force_collect_geometry(&mut process, 2);
    let mut ctx = live_context(&mut process, 2, &atoms);
    let result = super::string_bifs::bif_split(&[input, pattern, all], &mut ctx).expect("split");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    let mut parts = Vec::new();
    let mut current = result;
    while let Some(cons) = crate::term::boxed::Cons::new(current) {
        parts.push(result_bytes(cons.head()));
        current = cons.tail();
    }
    assert_eq!(parts, vec![b"aa".to_vec(), b"bb".to_vec(), b"cc".to_vec()]);
}

#[test]
fn string_find_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let input = inline_input(&mut process, b"abc\xE2\x80\x94def");
    let pattern = {
        let mut ctx = live_context(&mut process, 1, &atoms);
        let p = ctx.alloc_binary(b"\xE2\x80\x94").expect("pattern");
        ctx.detach_process();
        p
    };
    process.set_x_reg(1, pattern);
    force_collect_geometry(&mut process, 6);
    let mut ctx = live_context(&mut process, 2, &atoms);
    let result = super::string_bifs::bif_find(&[input, pattern], &mut ctx).expect("find");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    assert_eq!(result_bytes(result), b"\xE2\x80\x94def");
}

#[test]
fn string_pad_early_return_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let input = inline_input(&mut process, b"already long enough \xE2\x80\x94");
    let (length, direction, pad) = {
        let mut ctx = live_context(&mut process, 1, &atoms);
        let direction = Term::atom(atoms.intern("trailing"));
        let pad = ctx.alloc_binary(b" ").expect("pad");
        ctx.detach_process();
        (Term::small_int(3), direction, pad)
    };
    process.set_x_reg(1, pad);
    force_collect_geometry(&mut process, 23);
    let mut ctx = live_context(&mut process, 2, &atoms);
    let result =
        super::string_bifs::bif_pad(&[input, length, direction, pad], &mut ctx).expect("pad");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    assert_eq!(result_bytes(result), b"already long enough \xE2\x80\x94");
}

#[test]
fn string_slice_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let input = inline_input(&mut process, b"0123\xE2\x80\x94abcdef");
    force_collect_geometry(&mut process, 5);
    let mut ctx = live_context(&mut process, 1, &atoms);
    let result =
        super::string_bifs::bif_slice(&[input, Term::small_int(2), Term::small_int(4)], &mut ctx)
            .expect("slice");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    assert_eq!(result_bytes(result), b"23\xE2\x80\x94a");
}

#[test]
fn gate3_binary_part_owned_copy_survives_forced_collection() {
    // CONSUMER-LOAD-BEARING tripwire (asbytes-sweep AUDIT.md SAFE verdict at
    // gate3_bifs/additional.rs; the endorsed AION-DROPSTART-HARDENING lane
    // rerouted aion onto this variant): gate3's erlang:binary_part/3 owns its
    // slice BEFORE the allocating call, so it stays green under exactly the
    // geometry that reds the unfixed sites. The wall exists so a refactor
    // that removes the owned copy breaks a test instead of a production
    // system.
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let raw: Vec<u8> = (1..=40).collect();
    let input = inline_input(&mut process, &raw);
    force_collect_geometry(&mut process, 20);
    let mut ctx = live_context(&mut process, 1, &atoms);
    let result = super::super::gate3_bifs::bif_binary_part(
        &[input, Term::small_int(10), Term::small_int(20)],
        &mut ctx,
    )
    .expect("gate3 binary_part");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    let expected: Vec<u8> = (11..=30).collect();
    assert_eq!(result_bytes(result), expected);
}

/// ETF payload for `[<<"aaaa">>, <<"bbbb">>]` — 25 bytes, inline (≤ 64 B),
/// so the source lives on the young heap and moves under collection. Two
/// binary elements make the multi-slice partial-corruption shape: the first
/// element's allocation collecting invalidates every later read of the
/// source.
fn etf_two_binaries_payload() -> Vec<u8> {
    use crate::etf::tags;
    let mut payload = vec![tags::VERSION, tags::LIST_EXT, 0, 0, 0, 2];
    payload.extend_from_slice(&[tags::BINARY_EXT, 0, 0, 0, 4]);
    payload.extend_from_slice(b"aaaa");
    payload.extend_from_slice(&[tags::BINARY_EXT, 0, 0, 0, 4]);
    payload.extend_from_slice(b"bbbb");
    payload.push(tags::NIL_EXT);
    payload
}

fn cons_list_bytes(list: Term) -> Vec<Vec<u8>> {
    let mut parts = Vec::new();
    let mut current = list;
    while let Some(cons) = crate::term::boxed::Cons::new(current) {
        parts.push(result_bytes(cons.head()));
        current = cons.tail();
    }
    parts
}

#[test]
fn binary_to_term_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let payload = etf_two_binaries_payload();
    let input = inline_input(&mut process, &payload);
    force_collect_geometry(&mut process, 1);
    let mut ctx = live_context(&mut process, 1, &atoms);
    let result =
        super::super::etf_bifs::bif_binary_to_term(&[input], &mut ctx).expect("binary_to_term");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    assert_eq!(
        cons_list_bytes(result),
        vec![b"aaaa".to_vec(), b"bbbb".to_vec()]
    );
}

#[test]
fn binary_to_term_2_used_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let payload = etf_two_binaries_payload();
    let input = inline_input(&mut process, &payload);
    let options = {
        let mut ctx = live_context(&mut process, 1, &atoms);
        let used = Term::atom(atoms.intern("used"));
        let opts = ctx.alloc_cons(used, Term::NIL).expect("options list");
        ctx.detach_process();
        opts
    };
    process.set_x_reg(1, options);
    force_collect_geometry(&mut process, 1);
    let mut ctx = live_context(&mut process, 2, &atoms);
    let result = super::super::etf_bifs::bif_binary_to_term_2(&[input, options], &mut ctx)
        .expect("binary_to_term/2");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    let tuple = Tuple::new(result).expect("{term, used} tuple");
    let used = tuple.get(1).expect("used element");
    assert_eq!(
        used.as_small_int(),
        Some(i64::try_from(payload.len()).expect("payload length"))
    );
    let decoded = tuple.get(0).expect("decoded term");
    assert_eq!(
        cons_list_bytes(decoded),
        vec![b"aaaa".to_vec(), b"bbbb".to_vec()]
    );
}

// `uri_string:parse` map keys are ATOMS interned in the BIF context's table;
// look values up by atom term, which only works with the shared table above.
fn map_atom_value_bytes(map_term: Term, atoms: &AtomTable, name: &str) -> Option<Vec<u8>> {
    let key = Term::atom(atoms.intern(name));
    let map = crate::term::boxed::Map::new(map_term)?;
    map.get(key).map(result_bytes)
}

#[test]
fn uri_parse_components_survive_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let input = inline_input(&mut process, b"http://hh/pp?q=1#frag");
    force_collect_geometry(&mut process, 6);
    let mut ctx = live_context(&mut process, 1, &atoms);
    let result = super::uri_bifs::bif_uri_string_parse(&[input], &mut ctx).expect("uri parse");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    assert_eq!(
        map_atom_value_bytes(result, &atoms, "scheme").as_deref(),
        Some(&b"http"[..])
    );
    assert_eq!(
        map_atom_value_bytes(result, &atoms, "host").as_deref(),
        Some(&b"hh"[..])
    );
    assert_eq!(
        map_atom_value_bytes(result, &atoms, "path").as_deref(),
        Some(&b"/pp"[..])
    );
    assert_eq!(
        map_atom_value_bytes(result, &atoms, "query").as_deref(),
        Some(&b"q=1"[..])
    );
    assert_eq!(
        map_atom_value_bytes(result, &atoms, "fragment").as_deref(),
        Some(&b"frag"[..])
    );
}

#[test]
fn uri_dissect_query_error_detail_survives_forced_collection() {
    let mut process = Process::new(1, 256);
    let atoms = shared_atoms();
    let input = inline_input(&mut process, b"ok=1&bad=%ZZ\xE2\x80\x94tail");
    force_collect_geometry(&mut process, 13);
    let mut ctx = live_context(&mut process, 1, &atoms);
    // OTP contract (uri_string:dissect_query/1 spec): the failure face is the
    // RETURN VALUE `{error, Atom :: atom(), Term :: term()}` — QueryList |
    // {error, ...} — not a raised exception.
    let result = super::uri_bifs::bif_uri_string_dissect_query(&[input], &mut ctx)
        .expect("dissect_query returns the error tuple as a value");
    drop(ctx);
    assert!(
        process.heap().old_used() > 0,
        "geometry must have collected"
    );
    let tuple = crate::term::boxed::Tuple::new(result).expect("error tuple");
    assert_eq!(
        tuple.get(1),
        Some(Term::atom(atoms.intern("invalid_query")))
    );
    let detail = tuple.get(2).expect("detail element");
    assert_eq!(result_bytes(detail), b"bad=%ZZ\xE2\x80\x94tail");
}
