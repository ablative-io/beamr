//! Erlang bitwise BIFs.

use crate::atom::Atom;
use crate::native::ProcessContext;
use crate::term::Term;

pub fn bif_band(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    two_ints(args, |left, right| left & right)
}

pub fn bif_bnot(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [value] = args else {
        return Err(badarg());
    };
    let value = value.as_small_int().ok_or_else(badarg)?;
    Term::try_small_int(!value).ok_or_else(badarg)
}

pub fn bif_bor(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    two_ints(args, |left, right| left | right)
}

pub fn bif_bsl(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [value, shift] = args else {
        return Err(badarg());
    };
    let value = value.as_small_int().ok_or_else(badarg)?;
    let shift = shift
        .as_small_int()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(badarg)?;
    value
        .checked_shl(shift)
        .and_then(Term::try_small_int)
        .ok_or_else(badarg)
}

pub fn bif_bsr(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    let [value, shift] = args else {
        return Err(badarg());
    };
    let value = value.as_small_int().ok_or_else(badarg)?;
    let shift = shift
        .as_small_int()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(badarg)?;
    value
        .checked_shr(shift)
        .and_then(Term::try_small_int)
        .ok_or_else(badarg)
}

pub fn bif_bxor(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let _ = context;
    two_ints(args, |left, right| left ^ right)
}

fn two_ints(args: &[Term], operation: fn(i64, i64) -> i64) -> Result<Term, Term> {
    let [left, right] = args else {
        return Err(badarg());
    };
    let left = left.as_small_int().ok_or_else(badarg)?;
    let right = right.as_small_int().ok_or_else(badarg)?;
    Term::try_small_int(operation(left, right)).ok_or_else(badarg)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}
