//! Process dictionary BIFs.
//!
//! The dictionary is process-local mutable state, so these BIFs require a
//! heap-backed [`ProcessContext`] attached to the calling process.

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

/// Registers process dictionary BIFs into the VM-owned BIF registry.
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

/// erlang:put/2 — insert or replace a process dictionary entry.
pub fn bif_put(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [key, value] = args else {
        return Err(badarg());
    };

    context.dict_put(*key, *value)
}

/// erlang:get/1 — fetch a process dictionary value by key.
pub fn bif_get_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [key] = args else {
        return Err(badarg());
    };

    context.dict_get(*key)
}

/// erlang:get/0 — return all process dictionary entries as `{Key, Value}` tuples.
pub fn bif_get_0(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if !args.is_empty() {
        return Err(badarg());
    }

    let entries = context.dict_get_all()?;
    dictionary_entries_to_list(entries, context)
}

/// erlang:erase/1 — erase a process dictionary entry by key.
pub fn bif_erase_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [key] = args else {
        return Err(badarg());
    };

    context.dict_erase(*key)
}

/// erlang:erase/0 — erase and return all process dictionary entries.
pub fn bif_erase_0(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if !args.is_empty() {
        return Err(badarg());
    }

    let entries = context.dict_erase_all()?;
    dictionary_entries_to_list(entries, context)
}

/// erlang:get_keys/1 — return dictionary keys whose values exactly match `Value`.
pub fn bif_get_keys_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [value] = args else {
        return Err(badarg());
    };

    let keys = context.dict_get_keys(*value)?;
    context.alloc_list(&keys)
}

fn dictionary_entries_to_list(
    entries: Vec<(Term, Term)>,
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    let mut pairs = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        pairs.push(context.alloc_tuple(&[key, value])?);
    }
    context.alloc_list(&pairs)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod tests {
    use super::{bif_erase_1, bif_get_0, bif_get_1, bif_put, register_dictionary_bifs};
    use crate::atom::{Atom, AtomTable};
    use crate::native::{BifRegistryImpl, Capability, ProcessContext};
    use crate::process::Process;
    use crate::term::Term;
    use crate::term::boxed::{Cons, Tuple};

    fn context_with_process(process: &mut Process) -> ProcessContext<'_> {
        let mut context = ProcessContext::new();
        context.attach_process(process, 0);
        context
    }

    fn list_to_vec(list: Term) -> Vec<Term> {
        let mut elements = Vec::new();
        let mut tail = list;
        while !tail.is_nil() {
            let cons = Cons::new(tail).expect("proper list cell");
            elements.push(cons.head());
            tail = cons.tail();
        }
        elements
    }

    #[test]
    fn register_dictionary_bifs_registers_process_local_entries() {
        let atom_table = AtomTable::with_common_atoms();
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
            let function = atom_table.intern(name);
            let entry = registry
                .lookup(erlang, function, arity)
                .expect("registered BIF");
            assert_eq!(entry.capability, Capability::ProcessLocal);
        }
    }

    #[test]
    fn bif_put_get_erase_round_trip() {
        let mut process = Process::new(1, 32);
        let mut context = context_with_process(&mut process);
        let key = Term::atom(Atom::OK);
        let value = Term::small_int(99);

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
    fn bif_get_0_returns_complete_dictionary_as_tuple_list() {
        let mut process = Process::new(1, 64);
        let mut context = context_with_process(&mut process);
        let key_a = Term::atom(Atom::OK);
        let value_a = Term::small_int(1);
        let key_b = Term::atom(Atom::ERROR);
        let value_b = Term::small_int(2);
        bif_put(&[key_a, value_a], &mut context).expect("put a");
        bif_put(&[key_b, value_b], &mut context).expect("put b");

        let list = bif_get_0(&[], &mut context).expect("get/0");
        let pairs = list_to_vec(list);

        assert_eq!(pairs.len(), 2);
        let first = Tuple::new(pairs[0]).expect("first pair tuple");
        let second = Tuple::new(pairs[1]).expect("second pair tuple");
        assert_eq!(first.arity(), 2);
        assert_eq!(second.arity(), 2);
        assert_eq!(first.get(0), Some(key_a));
        assert_eq!(first.get(1), Some(value_a));
        assert_eq!(second.get(0), Some(key_b));
        assert_eq!(second.get(1), Some(value_b));
    }
}
