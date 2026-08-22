//! Map runtime helpers callable from JIT-generated code.

use crate::process::Process;
use crate::term::Term;
use crate::term::boxed::{Map, write_map};
use crate::term::compare;

use super::ir_exceptions::JitReturn;
use super::runtime::{alloc_words_rooted, process_from_abi};

pub(crate) extern "C" fn jit_map_new(
    process: *mut Process,
    source: u64,
    pairs: *const u64,
    pair_count: u64,
) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 0;
    };
    map_update(process, Term::from_raw(source), pairs, pair_count, false)
}

pub(crate) extern "C" fn jit_map_update(
    process: *mut Process,
    source: u64,
    pairs: *const u64,
    pair_count: u64,
) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 0;
    };
    map_update(process, Term::from_raw(source), pairs, pair_count, true)
}

pub(crate) extern "C" fn jit_map_get(map: u64, key: u64) -> JitReturn {
    let Some(map) = Map::new(Term::from_raw(map)) else {
        return map_get_return(0, 0);
    };
    map.get(Term::from_raw(key)).map_or_else(
        || map_get_return(0, 0),
        |value| map_get_return(1, value.raw()),
    )
}

pub(crate) extern "C" fn jit_map_has_key(map: u64, key: u64) -> u8 {
    Map::new(Term::from_raw(map))
        .and_then(|map| map.get(Term::from_raw(key)))
        .map_or(0, |_| 1)
}

fn map_update(
    process: &mut Process,
    source: Term,
    pairs: *const u64,
    pair_count: u64,
    exact: bool,
) -> u64 {
    let Some(source_map) = Map::new(source) else {
        return 0;
    };
    let Some(updates) = map_pair_terms(pairs, pair_count) else {
        return 0;
    };
    let Some(mut entries) = map_entries(source_map) else {
        return 0;
    };
    if exact
        && updates.iter().any(|(key, _value)| {
            entries
                .iter()
                .all(|(existing_key, _value)| *existing_key != *key)
        })
    {
        return 0;
    }
    for (key, value) in updates {
        if let Some((_existing_key, existing_value)) = entries
            .iter_mut()
            .find(|(existing_key, _value)| *existing_key == key)
        {
            *existing_value = value;
        } else {
            entries.push((key, value));
        }
    }
    entries.sort_by(|(left, _), (right, _)| compare::raw_cmp(*left, *right));
    write_map_entries(process, &entries).map_or(0, Term::raw)
}

fn map_pair_terms(pairs: *const u64, pair_count: u64) -> Option<Vec<(Term, Term)>> {
    let pair_count = usize::try_from(pair_count).ok()?;
    let term_count = pair_count.checked_mul(2)?;
    if term_count > 0 && pairs.is_null() {
        return None;
    }
    let raw_pairs = if term_count == 0 {
        &[]
    } else {
        // SAFETY: Generated code passes a stack slot containing exactly
        // `pair_count * 2` raw term words for the duration of this helper call.
        unsafe { std::slice::from_raw_parts(pairs, term_count) }
    };
    Some(
        raw_pairs
            .chunks_exact(2)
            .map(|pair| (Term::from_raw(pair[0]), Term::from_raw(pair[1])))
            .collect(),
    )
}

const fn map_get_return(status: u8, value: u64) -> JitReturn {
    JitReturn {
        status,
        _padding: [0; 7],
        value,
    }
}

fn map_entries(map: Map) -> Option<Vec<(Term, Term)>> {
    let mut entries = Vec::with_capacity(map.len());
    for index in 0..map.len() {
        entries.push((map.key(index)?, map.value(index)?));
    }
    Some(entries)
}

fn write_map_entries(process: &mut Process, entries: &[(Term, Term)]) -> Option<Term> {
    let words = map_word_count(entries.len())?;
    // Every key and value is rooted across the allocation. They live in
    // Rust-owned vectors, which are not GC roots, and the reservation inside
    // `alloc_words_rooted` can collect and move every one of them; writing the
    // pre-move copies into the fresh map is H4.
    //
    // Hoisting the reservation alone would NOT be enough here. The interpreter
    // twin (`put_map`, `interpreter/opcodes/closures.rs:476-480`) re-reads the
    // source map from its operand after reserving; this helper has no operand
    // to re-read, and generated code stages the update pairs into a Cranelift
    // stack slot the collector cannot see. Rooting is the only mechanism that
    // yields post-collection values here.
    let mut roots = entries
        .iter()
        .flat_map(|(key, value)| [*key, *value])
        .collect::<Vec<_>>();
    let ptr = alloc_words_rooted(process, words, &mut roots);
    if ptr.is_null() {
        return None;
    }
    let keys = roots.iter().step_by(2).copied().collect::<Vec<_>>();
    let values = roots.iter().skip(1).step_by(2).copied().collect::<Vec<_>>();
    // SAFETY: `alloc_words_rooted` returned a non-null pointer to exactly
    // `words` heap words owned by `process` for the duration of this call.
    let heap = unsafe { std::slice::from_raw_parts_mut(ptr, words) };
    write_map(heap, &keys, &values)
}

fn map_word_count(entries: usize) -> Option<usize> {
    entries.checked_mul(2)?.checked_add(2)
}

#[cfg(test)]
mod gc_hazard_tests {
    use super::*;
    use crate::atom::AtomTable;
    use crate::native::ProcessContext;
    use crate::term::binary_ref::BinaryRef;
    use crate::term::heap_borrow::HeapBorrow;
    use std::sync::Arc;

    fn test_context(process: &mut Process, live_x: u16) -> ProcessContext<'_> {
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::new(AtomTable::with_common_atoms())));
        context.attach_process(process, usize::from(live_x));
        context
    }

    /// Fills the nursery with live cons cells until fewer than `needed` words
    /// remain, so the next allocation of that size must collect. Never
    /// collects itself (it stops while `needed` still fits).
    fn fill_until(process: &mut Process, needed: usize) {
        let mut ctx = test_context(process, 4);
        while ctx.process_heap().expect("heap").available() >= needed {
            ctx.alloc_cons(Term::small_int(1), Term::NIL)
                .expect("filler");
        }
    }

    fn binary_bytes(term: Term, heap: HeapBorrow<'_>) -> Vec<u8> {
        BinaryRef::new(term)
            .expect("map value must stay a readable binary")
            .as_bytes(heap)
            .to_vec()
    }

    /// W3 (H4). Guards `write_map_entries` against reverting to the shape it
    /// had at `4055cbe`, where the key and value `Term`s were copied into
    /// Rust-owned vectors BEFORE the collecting reservation and those
    /// pre-collection words were then written into the freshly allocated map.
    /// Red evidence for that shape: the wall commit on this lane.
    ///
    /// The vectors are neither register- nor `native_roots`-resident, so a
    /// collection cannot forward them on its own. Boxed values make the
    /// corruption visible as bytes: the merged map ends up pointing into the
    /// zero-filled young region.
    ///
    /// The update value is parked in X1 deliberately — it mirrors the real
    /// lowering, where the pair terms are ALSO live in the register file and
    /// are forwarded there, while the copy the helper actually writes is not.
    #[test]
    fn map_update_boxed_values_survive_forced_collection() {
        let mut process = Process::new(1, 256);
        let original: Vec<Vec<u8>> = (0..3)
            .map(|slot: u8| (0..24).map(|byte| slot * 100 + byte).collect())
            .collect();
        let replacement: Vec<u8> = (200..224).collect();

        let (source_map, update_value) = {
            let mut ctx = test_context(&mut process, 0);
            let values: Vec<Term> = original
                .iter()
                .map(|bytes| ctx.alloc_binary(bytes).expect("inline value"))
                .collect();
            let keys: Vec<Term> = (0..3i64).map(Term::small_int).collect();
            let map = ctx.alloc_map(&keys, &values).expect("source map");
            let update_value = ctx.alloc_binary(&replacement).expect("update value");
            (map, update_value)
        };
        process.set_x_reg(0, source_map);
        process.set_x_reg(1, update_value);

        // The merged map keeps three entries, so the helper reserves
        // `3 * 2 + 2` words.
        let needed = map_word_count(3).expect("word count");
        fill_until(&mut process, needed);
        assert!(
            process.heap().available() < needed,
            "geometry must force the map allocation to collect"
        );
        assert_eq!(
            process.heap().old_used(),
            0,
            "nothing may be promoted before the subject call"
        );

        let pairs = [Term::small_int(1).raw(), update_value.raw()];
        let out_raw = jit_map_update(&mut process, source_map.raw(), pairs.as_ptr(), 1);
        assert_ne!(out_raw, 0, "map update must succeed");
        assert!(
            process.heap().old_used() > 0,
            "the map allocation must have run a collection"
        );
        assert_ne!(
            process.x_reg(0),
            source_map,
            "live source map should be promoted by the collection"
        );

        let merged = Map::new(Term::from_raw(out_raw)).expect("result must be a map");
        assert_eq!(merged.len(), 3, "entry count is unchanged by the update");

        // The direct face of H4: every value the merged map stores must be the
        // FORWARDED term. The forwarded values are readable from the promoted
        // source map (X0) and the promoted update value (X1). Raws are reported
        // so the evidence log carries the observed wrong terms themselves.
        let promoted = Map::new(process.x_reg(0)).expect("promoted source map");
        let forwarded: [Term; 3] = [
            promoted.value(0).expect("promoted value 0"),
            process.x_reg(1),
            promoted.value(2).expect("promoted value 2"),
        ];
        for (index, want) in forwarded.iter().enumerate() {
            let key = Term::small_int(index as i64);
            let got = merged.get(key).expect("every original key must survive");
            assert_eq!(
                got,
                *want,
                "merged map stored a pre-move value for key {index}: stored={:#018x} forwarded={:#018x}",
                got.raw(),
                want.raw()
            );
        }

        let expected: [&[u8]; 3] = [&original[0], &replacement, &original[2]];
        for (index, want) in expected.iter().enumerate() {
            let key = Term::small_int(index as i64);
            let value = merged.get(key).expect("every original key must survive");
            assert_eq!(
                binary_bytes(value, process.borrow_terms()),
                *want,
                "value for key {index} must read back byte-exact"
            );
        }
    }
}
