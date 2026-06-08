//! TCP socket option BIFs.

use crate::atom::{Atom, AtomTable};
use crate::io::resource::{FdMode, FdResource};
use crate::native::{BifRegistryImpl, Capability, NativeRegistrationError, ProcessContext};
use crate::term::Term;
use crate::term::boxed::{Cons, Tuple};

const ACTIVE_READ_BUFFER_BYTES: usize = 64 * 1024;

/// Registers Erlang TCP socket BIFs.
pub fn register_tcp_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let erlang = atom_table.intern("erlang");
    for (name, arity, function) in [
        ("tcp_setopts", 2, tcp_setopts as crate::native::NativeFn),
        (
            "tcp_controlling_process",
            2,
            tcp_controlling_process as crate::native::NativeFn,
        ),
    ] {
        registry.register(
            erlang,
            atom_table.intern(name),
            arity,
            function,
            Capability::ExternalIo,
        )?;
    }
    Ok(())
}

/// erlang:tcp_setopts/2.
pub fn tcp_setopts(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [socket_term, options] = args else {
        return Err(badarg());
    };
    let resource = FdResource::new(*socket_term).ok_or_else(badarg)?;
    let requested_mode = parse_active_option(*options, context)?;
    let previous_mode = resource.mode();
    resource.set_mode(requested_mode);
    if matches!(requested_mode, FdMode::Active | FdMode::ActiveOnce)
        && previous_mode == FdMode::Passive
    {
        let facility = context.tcp_io_facility().ok_or_else(badarg)?;
        let _submitted =
            facility.submit_active_tcp_read(resource.inner(), ACTIVE_READ_BUFFER_BYTES);
    }
    Ok(Term::atom(Atom::OK))
}

/// erlang:tcp_controlling_process/2.
pub fn tcp_controlling_process(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [socket_term, new_pid_term] = args else {
        return Err(badarg());
    };
    let resource = FdResource::new(*socket_term).ok_or_else(badarg)?;
    let new_pid = new_pid_term.as_pid().ok_or_else(badarg)?;
    let caller = context.pid().ok_or_else(badarg)?;
    if resource.controlling_process() != caller {
        let not_owner = context.atom_table().ok_or_else(badarg)?.intern("not_owner");
        return context.alloc_tuple(&[Term::atom(Atom::ERROR), Term::atom(not_owner)]);
    }
    resource.set_controlling_process(new_pid);
    Ok(Term::atom(Atom::OK))
}

fn parse_active_option(options: Term, context: &ProcessContext) -> Result<FdMode, Term> {
    let active_atom = context.atom_table().ok_or_else(badarg)?.intern("active");
    let once_atom = context.atom_table().ok_or_else(badarg)?.intern("once");
    let mut mode = None;
    let mut tail = options;
    while tail != Term::NIL {
        let cons = Cons::new(tail).ok_or_else(badarg)?;
        let tuple = Tuple::new(cons.head()).ok_or_else(badarg)?;
        if tuple.arity() != 2 {
            return Err(badarg());
        }
        let key = tuple.get(0).ok_or_else(badarg)?;
        let value = tuple.get(1).ok_or_else(badarg)?;
        if key != Term::atom(active_atom) {
            return Err(badarg());
        }
        mode = Some(match value {
            atom if atom == Term::atom(Atom::TRUE) => FdMode::Active,
            atom if atom == Term::atom(Atom::FALSE) => FdMode::Passive,
            atom if atom == Term::atom(once_atom) => FdMode::ActiveOnce,
            _ => return Err(badarg()),
        });
        tail = cons.tail();
    }
    mode.ok_or_else(badarg)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}
