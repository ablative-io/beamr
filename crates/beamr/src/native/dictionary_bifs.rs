//! Process dictionary BIFs.
//!
//! The dictionary is private to the calling process and is exposed through the
//! standard `erlang` process-dictionary BIFs.

use crate::atom::{Atom, AtomTable};
use crate::native::{
    BifRegistryImpl, Capability, NativeFn, NativeRegistrationError, ProcessContext,
};
use crate::term::Term;

type DictionaryBif = (&'static str, u8, NativeFn);

const DICTIONARY_BIFS: &[DictionaryBif] = &[
    ("put", 2, bif_put),
    ("get", 1, bif_get_1),
    ("get", 0, bif_get_0),
    ("erase", 1, bif_erase_1),
    ("erase", 0, bif_erase_0),
    ("get_keys", 1, bif_get_keys_1),
];

/// Registers all process dictionary BIFs.
pub fn register_dictionary_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let erlang = atom_table.intern("erlang");

    for &(function_name, arity, native_function) in DICTIONARY_BIFS {
        let function = atom_table.intern(function_name);
        registry.register(
            erlang,
            function,
            arity,
            native_function,
            Capability::ProcessLocal,
        )?;
    }

    Ok(())
}

/// erlang:put/2 — stores `Value` under `Key`, returning the old value or `undefined`.
pub fn bif_put(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [key, value] = args else {
        return Err(badarg());
    };

    context.dict_put(*key, *value)
}

/// erlang:get/1 — returns the value for `Key`, or `undefined`.
pub fn bif_get_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [key] = args else {
        return Err(badarg());
    };

    context.dict_get(*key)
}

/// erlang:get/0 — returns the full process dictionary as a list of `{Key, Value}` tuples.
pub fn bif_get_0(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if !args.is_empty() {
        return Err(badarg());
    }

    let entry_count = context.dict_len()?;
    // Reserve while the entries are still rooted by the process dictionary. If
    // allocation triggered GC after copying boxed terms out of the dictionary,
    // those local copies would not be rewritten by GC.
    context.ensure_heap_space(entries_heap_words(entry_count))?;

    let entries = context.dict_get_all()?;
    entries_to_list(&entries, context)
}

/// erlang:erase/1 — removes `Key`, returning the old value or `undefined`.
pub fn bif_erase_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [key] = args else {
        return Err(badarg());
    };

    context.dict_erase(*key)
}

/// erlang:erase/0 — clears the dictionary and returns previous entries as tuples.
pub fn bif_erase_0(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if !args.is_empty() {
        return Err(badarg());
    }

    let entry_count = context.dict_len()?;
    // Preflight before draining so erased boxed keys/values remain rooted if GC
    // runs to make result space available. Once this succeeds, allocation below
    // cannot trigger GC and the returned Vec can safely hold copied terms.
    context.ensure_heap_space(entries_heap_words(entry_count))?;

    let entries = context.dict_erase_all()?;
    entries_to_list(&entries, context)
}

/// erlang:get_keys/1 — returns all keys whose value exactly matches the argument.
pub fn bif_get_keys_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [value] = args else {
        return Err(badarg());
    };

    let matching_key_count = context.dict_count_keys_for_value(*value)?;
    // Preflight while matching keys are still rooted by the dictionary. This
    // prevents GC from relocating boxed keys after they have been copied into a
    // temporary Vec that GC does not know how to rewrite.
    context.ensure_heap_space(list_heap_words(matching_key_count))?;

    let keys = context.dict_get_keys(*value)?;
    context.alloc_list(&keys)
}

const fn entries_heap_words(entry_count: usize) -> usize {
    entry_count * 5
}

const fn list_heap_words(element_count: usize) -> usize {
    element_count * 2
}

fn entries_to_list(entries: &[(Term, Term)], context: &mut ProcessContext) -> Result<Term, Term> {
    let mut tuples = Vec::with_capacity(entries.len());
    for &(key, value) in entries {
        tuples.push(context.alloc_tuple(&[key, value])?);
    }
    context.alloc_list(&tuples)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod tests {
    use super::{
        bif_erase_0, bif_erase_1, bif_get_0, bif_get_1, bif_get_keys_1, bif_put,
        register_dictionary_bifs,
    };
    use crate::atom::{Atom, AtomTable};
    use crate::native::{BifRegistryImpl, Capability, ProcessContext};
    use crate::process::Process;
    use crate::term::Term;
    use crate::term::boxed::{Cons, Tuple};

    fn context(process: &mut Process) -> ProcessContext<'_> {
        let mut context = ProcessContext::new();
        context.attach_process(process, 0);
        context
    }

    fn list_terms(list: Term) -> Vec<Term> {
        let mut values = Vec::new();
        let mut tail = list;
        while !tail.is_nil() {
            let cons = Cons::new(tail).expect("proper list cons");
            values.push(cons.head());
            tail = cons.tail();
        }
        values
    }

    /// AR-1 row 4, site 4 (`entries_to_list`) — arm A of the two-arm red probe.
    ///
    /// The ledger flags this site: `tuples` is a bare `Vec<Term>` accumulating
    /// across `alloc_tuple` calls, rooted only at the terminal `alloc_list`.
    /// The shape is real. Whether the DEFECT fires is a separate question, and
    /// this probe is what separates them.
    ///
    /// Arm A (here, unmodified production bytes) must be **GREEN**.
    /// Arm B — the `ensure_heap_space` prereserve in `bif_get_0` deleted — must
    /// be **RED**. Arm B is the positive control: without it, a green here
    /// cannot tell *"the prereserve saved the accumulator"* from *"no
    /// collection ever happened"*, which is the exact trap the sibling test
    /// `get_0_returns_complete_dictionary_as_tuple_list` sits in — it drives
    /// this same site with immediates and is structurally incapable of failing.
    ///
    /// ⛔ **THE VERDICT IS TWO-TIER, AND THE NAME CARRIES IT: the flagged shape
    /// IS present at this site, and what defends it is the CALLER'S prereserve,
    /// not anything the site does.** `bif_get_0` reserves `entry_count * 5`
    /// while the process dictionary still roots the entries, so the
    /// accumulation loop cannot collect. Arm D (`entry_count * 2`) is RED,
    /// which is what shows the 3N tuple component is the load-bearing part.
    /// A green here therefore means *this caller reserves enough today* — it
    /// does NOT mean `entries_to_list` is safe, and any new caller reaching it
    /// without that reserve reopens the site.
    #[test]
    fn ar1_site4_defended_by_the_callers_prereserve_not_by_the_site() {
        use crate::term::binary::Binary;

        // Measured, not guessed: at 200 entries of 24 bytes the nursery holds 22
        // words against the 1000 the result needs, so `bif_get_0`'s reserve is
        // unambiguously the thing that makes room. (40/24 leaves 312 words free
        // and CONTROL 1 correctly refuses it.)
        const ENTRIES: usize = 200;
        const PAYLOAD: usize = 24;

        let mut process = Process::new(1, 512);
        let mut context = context(&mut process);

        // Values are BINARIES, never immediates: an immediate needs no
        // allocation, so the accumulator would never be live across one.
        for index in 0..ENTRIES {
            let payload = vec![u8::try_from(index).expect("index fits a byte"); PAYLOAD];
            let value = context.alloc_binary(&payload).expect("value binary");
            bif_put(&[Term::small_int(index as i64), value], &mut context).expect("put");
        }

        // CONTROL 1 — the prereserve must be load-bearing. If the heap already
        // has room for the whole result, nothing would collect and a green
        // below would report the absence of pressure, not the presence of
        // safety.
        let available = context
            .process_heap()
            .expect("process attached")
            .available();
        // Emitted, not just asserted: the control reading is the evidence that
        // the prereserve was load-bearing on this run, so it has to be legible
        // under `--nocapture` rather than only on failure.
        println!(
            "CONTROL 1: {available} words available vs {} needed",
            ENTRIES * 5
        );
        assert!(
            available < ENTRIES * 5,
            "control: {available} words already available covers the {} the result needs, \
             so no collection can occur and this probe proves nothing",
            ENTRIES * 5
        );

        let list = bif_get_0(&[], &mut context).expect("get/0");

        // Compare by VALUE, never by Term identity: a Vec of Terms held by this
        // test would go stale across a collection exactly as the production
        // accumulator does. The payload bytes are the identity.
        let mut seen: Vec<u8> = list_terms(list)
            .into_iter()
            .map(|entry| {
                let (_key, value) = tuple_pair(entry);
                let bytes = Binary::new(value)
                    .expect("value survived as a binary")
                    .as_bytes();
                assert_eq!(bytes.len(), PAYLOAD, "value binary truncated");
                assert!(
                    bytes.iter().all(|byte| *byte == bytes[0]),
                    "value binary contents are not uniform — the accumulator went stale"
                );
                bytes[0]
            })
            .collect();
        seen.sort_unstable();

        let expected: Vec<u8> = (0..ENTRIES).map(|i| u8::try_from(i).unwrap()).collect();
        assert_eq!(seen, expected, "an entry was lost or corrupted");
    }

    fn tuple_pair(term: Term) -> (Term, Term) {
        let tuple = Tuple::new(term).expect("dictionary entry tuple");
        assert_eq!(tuple.arity(), 2);
        (
            tuple.get(0).expect("key element"),
            tuple.get(1).expect("value element"),
        )
    }

    #[test]
    fn dictionary_bifs_put_get_and_erase_round_trip() {
        let mut process = Process::new(1, 64);
        let mut context = context(&mut process);
        let key = Term::atom(Atom::OK);
        let value = Term::small_int(42);

        assert_eq!(
            bif_put(&[key, value], &mut context),
            Ok(Term::atom(Atom::UNDEFINED))
        );
        assert_eq!(bif_get_1(&[key], &mut context), Ok(value));
        assert_eq!(bif_erase_1(&[key], &mut context), Ok(value));
        assert_eq!(
            bif_get_1(&[key], &mut context),
            Ok(Term::atom(Atom::UNDEFINED))
        );
    }

    #[test]
    fn get_0_returns_complete_dictionary_as_tuple_list() {
        let mut process = Process::new(1, 128);
        let mut context = context(&mut process);
        bif_put(&[Term::atom(Atom::OK), Term::small_int(1)], &mut context).expect("put ok");
        bif_put(&[Term::atom(Atom::ERROR), Term::small_int(2)], &mut context).expect("put error");

        let list = bif_get_0(&[], &mut context).expect("get/0");
        let pairs: Vec<_> = list_terms(list).into_iter().map(tuple_pair).collect();

        assert_eq!(
            pairs,
            vec![
                (Term::atom(Atom::OK), Term::small_int(1)),
                (Term::atom(Atom::ERROR), Term::small_int(2)),
            ]
        );
    }

    #[test]
    fn get_0_preflights_heap_space_while_dictionary_roots_boxed_entries() {
        let mut process = Process::new(1, 8);
        let key = {
            let mut context = context(&mut process);
            context
                .alloc_tuple(&[Term::small_int(10)])
                .expect("key allocation")
        };
        {
            let mut context = context(&mut process);
            let _garbage = context
                .alloc_tuple(&[Term::small_int(11)])
                .expect("garbage allocation");
        }
        {
            let mut context = context(&mut process);
            bif_put(&[key, Term::small_int(1)], &mut context).expect("put boxed key");
        }

        let list = {
            let mut context = context(&mut process);
            bif_get_0(&[], &mut context).expect("get/0 after GC preflight")
        };
        let pair = list_terms(list)
            .into_iter()
            .map(tuple_pair)
            .next()
            .expect("one pair");

        assert_eq!(pair.0.raw(), process.dict_get_all()[0].0.raw());
    }

    #[test]
    fn erase_0_reports_allocation_failure_without_clearing_dictionary() {
        let mut process = Process::new(1, 8);
        process.heap_mut().set_max_capacity(8);
        let mut context = context(&mut process);

        bif_put(&[Term::atom(Atom::OK), Term::small_int(1)], &mut context).expect("put ok");
        bif_put(&[Term::atom(Atom::ERROR), Term::small_int(2)], &mut context).expect("put error");
        assert_eq!(
            bif_erase_0(&[], &mut context),
            Err(Term::atom(Atom::BADARG))
        );
        assert_eq!(
            bif_get_1(&[Term::atom(Atom::OK)], &mut context),
            Ok(Term::small_int(1))
        );
    }

    #[test]
    fn erase_0_returns_previous_entries_and_clears_dictionary() {
        let mut process = Process::new(1, 128);
        let mut context = context(&mut process);
        bif_put(&[Term::atom(Atom::OK), Term::small_int(1)], &mut context).expect("put ok");

        let list = bif_erase_0(&[], &mut context).expect("erase/0");
        let pairs: Vec<_> = list_terms(list).into_iter().map(tuple_pair).collect();

        assert_eq!(pairs, vec![(Term::atom(Atom::OK), Term::small_int(1))]);
        assert_eq!(bif_get_0(&[], &mut context), Ok(Term::NIL));
    }

    #[test]
    fn get_keys_1_matches_values_with_exact_equality() {
        let mut process = Process::new(1, 128);
        let mut context = context(&mut process);
        bif_put(&[Term::atom(Atom::OK), Term::small_int(7)], &mut context).expect("put ok");
        bif_put(&[Term::atom(Atom::ERROR), Term::small_int(7)], &mut context).expect("put error");
        bif_put(
            &[Term::atom(Atom::UNDEFINED), Term::small_int(8)],
            &mut context,
        )
        .expect("put undefined");

        let keys = bif_get_keys_1(&[Term::small_int(7)], &mut context).expect("get_keys/1");

        assert_eq!(
            list_terms(keys),
            vec![Term::atom(Atom::OK), Term::atom(Atom::ERROR)]
        );
    }

    #[test]
    fn register_dictionary_bifs_registers_all_process_local() {
        let atom_table = AtomTable::new();
        let registry = BifRegistryImpl::new();
        register_dictionary_bifs(&registry, &atom_table).expect("dictionary registration");
        let erlang = atom_table.intern("erlang");

        for (name, arity) in [
            ("put", 2),
            ("get", 1),
            ("get", 0),
            ("erase", 1),
            ("erase", 0),
            ("get_keys", 1),
        ] {
            let entry = registry
                .lookup(erlang, atom_table.intern(name), arity)
                .expect("dictionary BIF registered");
            assert_eq!(entry.capability, Capability::ProcessLocal);
        }
    }
}
