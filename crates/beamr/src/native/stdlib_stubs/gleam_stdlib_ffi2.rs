//! Gleam stdlib print-family natives.
//!
//! These are the only `gleam_stdlib` functions beamr serves natively: beamr
//! owns the IO sink, so the bytecode bodies (which write through `io`) are
//! bypassed in favour of direct sink writes. Every other `gleam_stdlib`
//! function is served by the real compiled bytecode shipped with each Gleam
//! build — registering natives for them shadows that bytecode and is
//! forbidden (see the no-shadow guard tests).

use crate::atom::{Atom, AtomTable};
use crate::io_sink::IoStream;
use crate::native::ProcessContext;
use crate::term::Term;
use crate::term::binary_ref::BinaryRef;
use crate::term::format::format_term;

pub fn bif_print(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    write_print_args(args, context, false, IoStream::Out)
}

pub fn bif_print_error(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    // Stderr-flavoured Gleam wrappers carry the err stream tag (WPORT-5 R2
    // item 4). A stream-aware sink (the browser console sink) splits them;
    // every pre-existing threaded sink keeps the historical shared-sink
    // behaviour through the `write_stream` default.
    write_print_args(args, context, false, IoStream::Err)
}

pub fn bif_println(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    write_print_args(args, context, true, IoStream::Out)
}

pub fn bif_println_error(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    // Stderr-flavoured: err stream tag, same rationale as `bif_print_error`.
    write_print_args(args, context, true, IoStream::Err)
}

/// Writes the rendered argument to the IO sink and returns the `nil` atom,
/// matching `gleam_stdlib.erl`'s print family (`io:put_chars(...), nil`).
fn write_print_args(
    args: &[Term],
    context: &mut ProcessContext,
    newline: bool,
    stream: IoStream,
) -> Result<Term, Term> {
    let [value] = args else {
        return Err(badarg());
    };
    let mut bytes = print_bytes(*value, context);
    if newline {
        bytes.push(b'\n');
    }
    context.write_to_io_sink_tagged(stream, &bytes);
    Ok(Term::atom(Atom::NIL))
}

fn print_bytes(value: Term, context: &ProcessContext) -> Vec<u8> {
    BinaryRef::new(value)
        .map(|binary| binary.as_bytes().to_vec())
        .unwrap_or_else(|| render_term(value, context).into_bytes())
}

fn render_term(term: Term, context: &ProcessContext) -> String {
    let fallback = AtomTable::with_common_atoms();
    let atom_table = context.atom_table().unwrap_or(&fallback);
    format_term(term, atom_table)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}
