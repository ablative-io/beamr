use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use super::{BifRegistryImpl, Capability, NativeEntry, NativeReplacementError, ProcessContext};
use crate::atom::AtomTable;
use crate::term::Term;

fn forty_two(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    Ok(Term::small_int(42))
}

fn thirteen(_args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    Ok(Term::small_int(13))
}

#[test]
fn replace_existing_returns_delegatable_previous_entry_and_updates_dispatch() {
    let atoms = AtomTable::new();
    let registry = BifRegistryImpl::new();
    let erlang = atoms.intern("erlang");
    let function = atoms.intern("replacement_test");
    registry
        .register(erlang, function, 0, forty_two, Capability::Pure)
        .expect("register original BIF");

    let previous = registry
        .replace_existing(erlang, function, 0, thirteen, Capability::ProcessLocal)
        .expect("replace occupied MFA");

    assert_eq!(
        previous,
        NativeEntry {
            function: forty_two,
            dirty_kind: None,
            capability: Capability::Pure,
        }
    );
    let replacement = registry
        .lookup(erlang, function, 0)
        .expect("replacement remains registered");
    assert_eq!(
        (replacement.function)(&[], &mut ProcessContext::new()),
        Ok(Term::small_int(13))
    );
    assert_eq!(
        (previous.function)(&[], &mut ProcessContext::new()),
        Ok(Term::small_int(42)),
        "the returned original entry remains directly delegatable"
    );
}

#[test]
fn replace_existing_missing_mfa_is_typed_error_and_does_not_insert() {
    let atoms = AtomTable::new();
    let registry = BifRegistryImpl::new();
    let erlang = atoms.intern("erlang");
    let existing = atoms.intern("existing");
    let missing = atoms.intern("missing");
    registry
        .register(erlang, existing, 0, forty_two, Capability::Pure)
        .expect("register control BIF");
    let before = registry.registered_mfas();

    assert_eq!(
        registry.replace_existing(erlang, missing, 1, thirteen, Capability::Spawn),
        Err(NativeReplacementError::MissingMfa {
            module: erlang,
            function: missing,
            arity: 1,
        })
    );
    assert_eq!(registry.registered_mfas(), before);
    assert!(registry.lookup(erlang, missing, 1).is_none());
    assert_eq!(
        registry
            .lookup(erlang, existing, 0)
            .expect("control BIF remains unchanged"),
        NativeEntry {
            function: forty_two,
            dirty_kind: None,
            capability: Capability::Pure,
        }
    );
}

#[test]
fn concurrent_lookup_and_replacement_never_observe_a_torn_entry() {
    const READER_COUNT: usize = 4;
    const ITERATIONS: usize = 20_000;

    let atoms = AtomTable::new();
    let registry = Arc::new(BifRegistryImpl::new());
    let module = atoms.intern("atomicity");
    let function = atoms.intern("dispatch");
    registry
        .register(module, function, 0, forty_two, Capability::Pure)
        .expect("register original BIF");

    let original = NativeEntry {
        function: forty_two,
        dirty_kind: None,
        capability: Capability::Pure,
    };
    let replacement = NativeEntry {
        function: thirteen,
        dirty_kind: None,
        capability: Capability::Spawn,
    };
    let start = Arc::new(Barrier::new(READER_COUNT + 1));
    let concurrent_dispatches = Arc::new(AtomicUsize::new(0));

    thread::scope(|scope| {
        for _ in 0..READER_COUNT {
            let registry = Arc::clone(&registry);
            let start = Arc::clone(&start);
            let concurrent_dispatches = Arc::clone(&concurrent_dispatches);
            scope.spawn(move || {
                let mut context = ProcessContext::new();
                start.wait();
                for _ in 0..ITERATIONS {
                    let entry = registry
                        .lookup(module, function, 0)
                        .expect("occupied MFA cannot disappear");
                    let expected = if entry == original {
                        Term::small_int(42)
                    } else if entry == replacement {
                        Term::small_int(13)
                    } else {
                        panic!("lookup observed a torn NativeEntry: {entry:?}");
                    };
                    assert_eq!((entry.function)(&[], &mut context), Ok(expected));
                    concurrent_dispatches.fetch_add(1, Ordering::Relaxed);
                }
            });
        }

        let registry = Arc::clone(&registry);
        let start = Arc::clone(&start);
        scope.spawn(move || {
            start.wait();
            let mut expected_previous = original;
            for iteration in 0..ITERATIONS {
                let next = if iteration % 2 == 0 {
                    replacement
                } else {
                    original
                };
                let previous = registry
                    .replace_existing(module, function, 0, next.function, next.capability)
                    .expect("MFA remains occupied");
                assert_eq!(previous, expected_previous);
                expected_previous = next;
                if iteration % 64 == 0 {
                    thread::yield_now();
                }
            }
        });
    });

    assert_eq!(
        concurrent_dispatches.load(Ordering::Relaxed),
        READER_COUNT * ITERATIONS
    );
}
