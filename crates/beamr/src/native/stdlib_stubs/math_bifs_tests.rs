use crate::atom::Atom;
use crate::native::ProcessContext;
use crate::term::Term;
use crate::term::boxed::{Float, write_float};

use super::math_bifs::*;

fn context() -> ProcessContext {
    ProcessContext::new()
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

fn float(value: f64) -> Term {
    let heap = Box::leak(Box::new([0u64; 2]));
    write_float(heap, value).expect("float")
}

fn assert_float(term: Term, expected: f64) {
    assert_eq!(Float::new(term).expect("float").value(), expected);
}

#[test]
fn math_acceptance_values_match_expected_results() {
    let mut context = context();
    assert_float(bif_ceil(&[float(3.2)], &mut context).expect("ceil"), 4.0);
    assert_float(bif_floor(&[float(3.7)], &mut context).expect("floor"), 3.0);
    assert_float(
        bif_pow(&[float(2.0), float(10.0)], &mut context).expect("pow"),
        1024.0,
    );
    assert_float(bif_log(&[float(1.0)], &mut context).expect("log"), 0.0);
}

#[test]
fn exp_returns_boxed_float() {
    let mut context = context();
    let result = bif_exp(&[float(0.0)], &mut context).expect("exp");
    assert_float(result, 1.0);
}

#[test]
fn math_accepts_small_int_numbers() {
    let mut context = context();
    assert_float(
        bif_ceil(&[Term::small_int(3)], &mut context).expect("ceil"),
        3.0,
    );
}

#[test]
fn math_rejects_invalid_inputs() {
    let mut context = context();
    assert_eq!(
        bif_ceil(&[Term::atom(Atom::OK)], &mut context),
        Err(badarg())
    );
    assert_eq!(bif_log(&[float(0.0)], &mut context), Err(badarg()));
    assert_eq!(
        bif_pow(&[float(f64::INFINITY), float(2.0)], &mut context),
        Err(badarg())
    );
}
