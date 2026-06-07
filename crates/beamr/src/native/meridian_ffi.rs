//! Meridian workflow NIFs — Rust functions callable from Gleam workflows.
//!
//! Registered under the `meridian_ffi` module atom. These are proof-of-concept
//! implementations for testing the NIF wiring end-to-end.

use crate::atom::{Atom, AtomTable};
use crate::native::{BifRegistryImpl, NativeRegistrationError, ProcessContext};
use crate::term::Term;
use crate::term::binary::Binary;

pub fn register_meridian_ffi(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let module = atom_table.intern("meridian_ffi");
    registry.register(module, atom_table.intern("read_file"), 1, nif_read_file)?;
    registry.register(module, atom_table.intern("run_cmd"), 1, nif_run_cmd)?;
    registry.register(module, atom_table.intern("write_file"), 2, nif_write_file)?;
    registry.register(module, atom_table.intern("read_json"), 1, nif_read_json)?;
    registry.register(module, atom_table.intern("commit"), 1, nif_commit)?;
    registry.register(
        module,
        atom_table.intern("run_step_norn"),
        4,
        nif_run_step_norn,
    )?;
    Ok(())
}

fn ok_binary(ctx: &mut ProcessContext, content: &[u8]) -> Result<Term, Term> {
    ctx.alloc_binary_tuple(Atom::OK, content)
}

fn err_binary(ctx: &mut ProcessContext, reason: &[u8]) -> Term {
    ctx.alloc_binary_tuple(Atom::ERROR, reason)
        .unwrap_or_else(|_| Term::atom(Atom::ERROR))
}

fn ok_nil(ctx: &mut ProcessContext) -> Result<Term, Term> {
    ctx.alloc_tuple(&[Term::atom(Atom::OK), Term::NIL])
}

fn extract_string(term: Term) -> Result<String, Term> {
    let binary = Binary::new(term).ok_or(Term::atom(Atom::BADARG))?;
    String::from_utf8(binary.as_bytes().to_vec()).map_err(|_| Term::atom(Atom::BADARG))
}

fn nif_read_file(args: &[Term], ctx: &mut ProcessContext) -> Result<Term, Term> {
    let path = extract_string(args[0])?;
    match std::fs::read(&path) {
        Ok(content) => ok_binary(ctx, &content),
        Err(e) => Err(err_binary(ctx, e.to_string().as_bytes())),
    }
}

fn nif_run_cmd(args: &[Term], ctx: &mut ProcessContext) -> Result<Term, Term> {
    let command = extract_string(args[0])?;
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .output()
    {
        Ok(output) => ok_binary(ctx, &output.stdout),
        Err(e) => Err(err_binary(ctx, e.to_string().as_bytes())),
    }
}

fn nif_write_file(args: &[Term], ctx: &mut ProcessContext) -> Result<Term, Term> {
    let path = extract_string(args[0])?;
    let content = extract_string(args[1])?;
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, &content) {
        Ok(()) => ok_nil(ctx),
        Err(e) => Err(err_binary(ctx, e.to_string().as_bytes())),
    }
}

fn nif_read_json(args: &[Term], ctx: &mut ProcessContext) -> Result<Term, Term> {
    let path = extract_string(args[0])?;
    match std::fs::read_to_string(&path) {
        Ok(content) => ok_binary(ctx, content.as_bytes()),
        Err(e) => Err(err_binary(ctx, e.to_string().as_bytes())),
    }
}

fn nif_commit(args: &[Term], ctx: &mut ProcessContext) -> Result<Term, Term> {
    let _message = extract_string(args[0])?;
    ok_binary(ctx, b"commit stub")
}

fn nif_run_step_norn(args: &[Term], ctx: &mut ProcessContext) -> Result<Term, Term> {
    let _name = extract_string(args[0])?;
    let _profile = extract_string(args[1])?;
    let _instruction = extract_string(args[2])?;
    let _schema = extract_string(args[3])?;
    ok_binary(ctx, b"step stub")
}
