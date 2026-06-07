use crate::atom::Atom;
use crate::native::ProcessContext;
use crate::process::Process;
use crate::term::Term;

use super::bitwise_bifs::*;

fn context(process: &mut Process) -> ProcessContext<'_> {
    let mut context = ProcessContext::new();
    context.attach_process(process, 0);
    context
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[test]
fn bitwise_acceptance_values_match_erlang() {
    let mut process = Process::new(1, 64);
    let mut context = context(&mut process);
    assert_eq!(
        bif_band(&[Term::small_int(5), Term::small_int(3)], &mut context),
        Ok(Term::small_int(1))
    );
    assert_eq!(
        bif_bor(&[Term::small_int(5), Term::small_int(3)], &mut context),
        Ok(Term::small_int(7))
    );
    assert_eq!(
        bif_bxor(&[Term::small_int(5), Term::small_int(3)], &mut context),
        Ok(Term::small_int(6))
    );
    assert_eq!(
        bif_bsl(&[Term::small_int(1), Term::small_int(4)], &mut context),
        Ok(Term::small_int(16))
    );
    assert_eq!(
        bif_bsr(&[Term::small_int(16), Term::small_int(4)], &mut context),
        Ok(Term::small_int(1))
    );
    assert_eq!(
        bif_bnot(&[Term::small_int(0)], &mut context),
        Ok(Term::small_int(-1))
    );
}

#[test]
fn bitwise_rejects_non_integer_arguments() {
    let mut process = Process::new(1, 64);
    let mut context = context(&mut process);
    assert_eq!(
        bif_band(&[Term::atom(Atom::OK), Term::small_int(1)], &mut context),
        Err(badarg())
    );
    assert_eq!(
        bif_bnot(&[Term::atom(Atom::OK)], &mut context),
        Err(badarg())
    );
    assert_eq!(
        bif_bor(&[Term::small_int(1), Term::atom(Atom::OK)], &mut context),
        Err(badarg())
    );
    assert_eq!(
        bif_bsl(&[Term::small_int(1), Term::small_int(-1)], &mut context),
        Err(badarg())
    );
    assert_eq!(
        bif_bsr(&[Term::small_int(1), Term::small_int(-1)], &mut context),
        Err(badarg())
    );
    assert_eq!(
        bif_bxor(&[Term::small_int(1), Term::atom(Atom::OK)], &mut context),
        Err(badarg())
    );
}
