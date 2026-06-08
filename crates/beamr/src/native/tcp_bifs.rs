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

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    use crate::io::resource::{FD_RESOURCE_WORDS, FdInner, write_fd_resource};
    use crate::native::TcpIoFacility;
    use crate::process::Process;
    use crate::process::heap::DEFAULT_HEAP_SIZE;
    use crate::term::boxed::{Tuple, write_cons, write_tuple};

    #[derive(Default)]
    struct MockTcpIoFacility {
        submissions: Mutex<Vec<(std::sync::Arc<FdInner>, usize)>>,
    }

    impl TcpIoFacility for MockTcpIoFacility {
        fn submit_active_tcp_read(
            &self,
            socket: std::sync::Arc<FdInner>,
            buf_len: usize,
        ) -> Option<u64> {
            let mut submissions = self
                .submissions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            submissions.push((socket, buf_len));
            Some(submissions.len() as u64)
        }
    }

    fn context_with_pid(pid: u64) -> (Arc<AtomTable>, ProcessContext<'static>) {
        let atom_table = Arc::new(AtomTable::new());
        let mut context = ProcessContext::new();
        context.set_pid(Some(pid));
        context.set_atom_table(Some(Arc::clone(&atom_table)));
        (atom_table, context)
    }

    fn socket_term(socket: Arc<FdInner>) -> (Vec<u64>, Term) {
        let mut heap = vec![0; FD_RESOURCE_WORDS];
        let term = write_fd_resource(&mut heap, socket).expect("fd resource term");
        (heap, term)
    }

    fn active_option_list(atom_table: &AtomTable, value: Term) -> (Vec<u64>, Vec<u64>, Term) {
        let active = atom_table.intern("active");
        let mut tuple_heap = vec![0; 3];
        let option =
            write_tuple(&mut tuple_heap, &[Term::atom(active), value]).expect("option tuple");
        let mut cons_heap = vec![0; 2];
        let list = write_cons(&mut cons_heap, option, Term::NIL).expect("option list");
        (tuple_heap, cons_heap, list)
    }

    #[test]
    fn tcp_setopts_active_from_passive_starts_read_loop() {
        let (atom_table, mut context) = context_with_pid(7);
        let facility = Arc::new(MockTcpIoFacility::default());
        context.set_tcp_io_facility(Some(facility.clone()));
        let socket = Arc::new(FdInner::new(55, 7));
        let (_socket_heap, socket_term) = socket_term(Arc::clone(&socket));
        let (_tuple_heap, _cons_heap, options) =
            active_option_list(&atom_table, Term::atom(Atom::TRUE));

        assert_eq!(
            tcp_setopts(&[socket_term, options], &mut context),
            Ok(Term::atom(Atom::OK))
        );
        assert_eq!(socket.mode(), FdMode::Active);
        let submissions = facility
            .submissions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(submissions.len(), 1);
        assert_eq!(submissions[0].0.fd(), 55);
        assert_eq!(submissions[0].1, ACTIVE_READ_BUFFER_BYTES);
    }

    #[test]
    fn tcp_setopts_active_once_from_active_does_not_start_duplicate_read() {
        let (atom_table, mut context) = context_with_pid(8);
        let facility = Arc::new(MockTcpIoFacility::default());
        context.set_tcp_io_facility(Some(facility.clone()));
        let socket = Arc::new(FdInner::new(56, 8));
        socket.set_mode(FdMode::Active);
        let (_socket_heap, socket_term) = socket_term(Arc::clone(&socket));
        let once = atom_table.intern("once");
        let (_tuple_heap, _cons_heap, options) = active_option_list(&atom_table, Term::atom(once));

        assert_eq!(
            tcp_setopts(&[socket_term, options], &mut context),
            Ok(Term::atom(Atom::OK))
        );
        assert_eq!(socket.mode(), FdMode::ActiveOnce);
        assert!(
            facility
                .submissions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        );
    }

    #[test]
    fn tcp_setopts_passive_stops_future_resubmits_without_facility() {
        let (atom_table, mut context) = context_with_pid(9);
        let socket = Arc::new(FdInner::new(57, 9));
        socket.set_mode(FdMode::Active);
        let (_socket_heap, socket_term) = socket_term(Arc::clone(&socket));
        let (_tuple_heap, _cons_heap, options) =
            active_option_list(&atom_table, Term::atom(Atom::FALSE));

        assert_eq!(
            tcp_setopts(&[socket_term, options], &mut context),
            Ok(Term::atom(Atom::OK))
        );
        assert_eq!(socket.mode(), FdMode::Passive);
    }

    #[test]
    fn tcp_controlling_process_transfers_only_from_current_controller() {
        let (atom_table, mut owner_context) = context_with_pid(10);
        let socket = Arc::new(FdInner::new(58, 10));
        let (_socket_heap, socket_term) = socket_term(Arc::clone(&socket));

        assert_eq!(
            tcp_controlling_process(&[socket_term, Term::pid(11)], &mut owner_context),
            Ok(Term::atom(Atom::OK))
        );
        assert_eq!(socket.controlling_process(), 11);

        let mut process = Process::new(12, DEFAULT_HEAP_SIZE);
        let mut not_owner_context = ProcessContext::new();
        not_owner_context.set_atom_table(Some(atom_table));
        not_owner_context.attach_process(&mut process, 0);
        let not_owner = not_owner_context
            .atom_table()
            .expect("atom table")
            .intern("not_owner");
        let result = tcp_controlling_process(&[socket_term, Term::pid(12)], &mut not_owner_context)
            .expect("not_owner tuple");
        let tuple = Tuple::new(result).expect("error tuple");
        assert_eq!(tuple.get(0), Some(Term::atom(Atom::ERROR)));
        assert_eq!(tuple.get(1), Some(Term::atom(not_owner)));
        assert_eq!(socket.controlling_process(), 11);
    }
}
