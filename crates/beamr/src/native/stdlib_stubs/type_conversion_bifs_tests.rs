use std::sync::Arc;

use crate::atom::{Atom, AtomTable};
use crate::native::ProcessContext;
use crate::process::Process;
use crate::term::Term;
use crate::term::binary::{self, Binary};
use crate::term::boxed::{Cons, Float, Tuple, write_cons, write_float, write_tuple};

use super::type_conversion_bifs::*;

fn context(process: &mut Process) -> ProcessContext<'_> {
    let mut context = ProcessContext::new();
    context.attach_process(process, 0);
    context
}

fn atom_context(process: &mut Process) -> ProcessContext<'_> {
    let table = Arc::new(AtomTable::with_common_atoms());
    let mut context = ProcessContext::new();
    context.set_atom_table(Some(table));
    context.attach_process(process, 0);
    context
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

fn binary(bytes: &[u8]) -> Term {
    let heap = Box::leak(vec![0u64; 2 + binary::packed_word_count(bytes.len())].into_boxed_slice());
    binary::write_binary(heap, bytes).expect("binary")
}

fn list(values: &[Term]) -> Term {
    let mut tail = Term::NIL;
    for value in values.iter().rev() {
        let heap = Box::leak(Box::new([0u64; 2]));
        tail = write_cons(heap, *value, tail).expect("cons");
    }
    tail
}

fn assert_binary(term: Term, expected: &[u8]) {
    let binary = Binary::new(term).expect("binary term");
    assert_eq!(binary.as_bytes(), expected);
}

fn list_to_vec(term: Term) -> Vec<Term> {
    let mut values = Vec::new();
    let mut current = term;
    while !current.is_nil() {
        let cons = Cons::new(current).expect("proper list");
        values.push(cons.head());
        current = cons.tail();
    }
    values
}

#[test]
fn atom_to_binary_converts_common_atom() {
    let mut process = Process::new(1, 128);
    let mut context = atom_context(&mut process);
    let result = bif_atom_to_binary(&[Term::atom(Atom::OK)], &mut context).expect("ok");
    assert_binary(result, b"ok");
}

#[test]
fn binary_to_float_parses_float_text() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    let result = bif_binary_to_float(&[binary(b"3.5")], &mut context).expect("float");
    assert_eq!(Float::new(result).expect("float").value(), 3.5);
}

#[test]
fn binary_to_integer_parses_decimal() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    assert_eq!(
        bif_binary_to_integer(&[binary(b"42")], &mut context),
        Ok(Term::small_int(42))
    );
}

#[test]
fn binary_to_integer_parses_radix() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    assert_eq!(
        bif_binary_to_integer_radix(&[binary(b"FF"), Term::small_int(16)], &mut context),
        Ok(Term::small_int(255))
    );
}

#[test]
fn float_converts_integer_and_preserves_float() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    let converted = bif_float(&[Term::small_int(7)], &mut context).expect("float");
    assert_eq!(Float::new(converted).expect("float").value(), 7.0);

    let mut heap = [0u64; 2];
    let existing = write_float(&mut heap, 2.5).expect("float");
    assert_eq!(bif_float(&[existing], &mut context), Ok(existing));
}

#[test]
fn integer_to_binary_formats_decimal() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    let result = bif_integer_to_binary(&[Term::small_int(42)], &mut context).expect("binary");
    assert_binary(result, b"42");
}

#[test]
fn integer_to_binary_formats_radix() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    let result =
        bif_integer_to_binary_radix(&[Term::small_int(255), Term::small_int(16)], &mut context)
            .expect("binary");
    assert_binary(result, b"FF");
}

#[test]
fn integer_to_list_formats_decimal_chars() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    let result = bif_integer_to_list(&[Term::small_int(-42)], &mut context).expect("list");
    assert_eq!(
        list_to_vec(result),
        vec![
            Term::small_int(45),
            Term::small_int(52),
            Term::small_int(50)
        ]
    );
}

#[test]
fn iolist_to_binary_flattens_byte_list_and_binary_chunks() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    let iolist = list(&[Term::small_int(65), binary(b"BC"), Term::small_int(68)]);
    let result = bif_iolist_to_binary(&[iolist], &mut context).expect("binary");
    assert_binary(result, b"ABCD");
}

#[test]
fn list_to_bitstring_returns_binary() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    let result = bif_list_to_bitstring(
        &[list(&[Term::small_int(1), Term::small_int(2)])],
        &mut context,
    )
    .expect("binary");
    assert_binary(result, &[1, 2]);
}

#[test]
fn list_to_tuple_converts_values() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    let result = bif_list_to_tuple(
        &[list(&[
            Term::small_int(1),
            Term::small_int(2),
            Term::small_int(3),
        ])],
        &mut context,
    )
    .expect("tuple");
    let tuple = Tuple::new(result).expect("tuple");
    assert_eq!(tuple.arity(), 3);
    assert_eq!(tuple.get(0), Some(Term::small_int(1)));
    assert_eq!(tuple.get(1), Some(Term::small_int(2)));
    assert_eq!(tuple.get(2), Some(Term::small_int(3)));
}

#[test]
fn tuple_to_list_converts_values() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    let mut heap = [0u64; 4];
    let tuple = write_tuple(
        &mut heap,
        &[Term::small_int(1), Term::small_int(2), Term::small_int(3)],
    )
    .expect("tuple");
    let result = bif_tuple_to_list(&[tuple], &mut context).expect("list");
    assert_eq!(
        list_to_vec(result),
        vec![Term::small_int(1), Term::small_int(2), Term::small_int(3)]
    );
}

#[test]
fn type_conversion_rejects_non_matching_types() {
    let mut process = Process::new(1, 128);
    let mut context = context(&mut process);
    assert_eq!(
        bif_binary_to_integer(&[Term::atom(Atom::OK)], &mut context),
        Err(badarg())
    );
    assert_eq!(
        bif_integer_to_binary(&[Term::atom(Atom::OK)], &mut context),
        Err(badarg())
    );
    assert_eq!(
        bif_list_to_tuple(&[Term::small_int(1)], &mut context),
        Err(badarg())
    );
    assert_eq!(
        bif_tuple_to_list(&[Term::small_int(1)], &mut context),
        Err(badarg())
    );
}
