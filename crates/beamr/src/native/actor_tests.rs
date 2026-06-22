//! Unit tests for the private envelope encode/decode of [`super`]
//! (the ergonomic native-actor layer). End-to-end behaviour through a real
//! scheduler is covered by `crates/beamr/tests/native_actor.rs`.

use super::{Incoming, TAG_CALL, TAG_CAST, TAG_REPLY, decode_reply, next_ref};
use crate::process::Process;
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::term::Term;
use crate::term::boxed::write_tuple;

fn build(process: &mut Process, elements: &[Term]) -> Term {
    let slice = process
        .heap_mut()
        .alloc_slice(1 + elements.len())
        .expect("heap space for test tuple");
    write_tuple(slice, elements).expect("tuple writes")
}

#[test]
fn refs_are_unique_and_monotonic() {
    let first = next_ref();
    let second = next_ref();
    assert!(second > first, "refs must be monotonically increasing");
}

#[test]
fn decodes_call_cast_and_reply_envelopes() {
    let mut process = Process::new(7, DEFAULT_HEAP_SIZE);

    let cast = build(
        &mut process,
        &[Term::small_int(TAG_CAST), Term::small_int(99)],
    );
    assert!(matches!(
        Incoming::decode(cast),
        Some(Incoming::Cast { request }) if request.as_small_int() == Some(99)
    ));

    let call = build(
        &mut process,
        &[
            Term::small_int(TAG_CALL),
            Term::small_int(5),
            // reply_to is an integer scalar, not a pid term (see `Incoming`).
            Term::small_int(3),
            Term::small_int(42),
        ],
    );
    match Incoming::decode(call) {
        Some(Incoming::Call {
            reference,
            reply_to,
            request,
        }) => {
            assert_eq!(reference, 5);
            assert_eq!(reply_to, 3);
            assert_eq!(request.as_small_int(), Some(42));
        }
        _ => panic!("expected a decoded call envelope"),
    }

    let reply = build(
        &mut process,
        &[
            Term::small_int(TAG_REPLY),
            Term::small_int(5),
            Term::small_int(84),
        ],
    );
    assert_eq!(decode_reply(reply), Some((5, Term::small_int(84))));
}

#[test]
fn non_envelope_terms_decode_to_none() {
    assert!(Incoming::decode(Term::small_int(1)).is_none());
    assert!(decode_reply(Term::atom(crate::atom::Atom::OK)).is_none());
}
