//! Process metadata BIFs.
//!
//! These BIFs expose BEAM process metadata that is owned by the scheduler, such
//! as group leader state, without implementing I/O protocol routing.

use crate::atom::{Atom, AtomTable};
use crate::native::{
    BifRegistryImpl, Capability, NativeFn, NativeRegistrationError, ProcessContext,
};
use crate::term::Term;

/// Scheduler-backed group leader operations used by process-info BIFs.
pub trait GroupLeaderFacility: Send + Sync {
    /// Return the group leader for `pid` when the process exists.
    fn group_leader(&self, pid: u64) -> Option<Term>;

    /// Set the group leader for `pid`. Returns false when `pid` does not exist.
    fn set_group_leader(&self, pid: u64, group_leader: Term) -> bool;
}

type ProcessInfoBif = (&'static str, u8, Capability, NativeFn);

const PROCESS_INFO_BIFS: &[ProcessInfoBif] = &[
    ("group_leader", 0, Capability::Pure, bif_group_leader_0),
    ("group_leader", 2, Capability::Pure, bif_group_leader_2),
];

/// Registers process metadata BIFs under the `erlang` module.
pub fn register_process_info_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let erlang = atom_table.intern("erlang");
    for &(function_name, arity, capability, native_function) in PROCESS_INFO_BIFS {
        let function = atom_table.intern(function_name);
        registry.register(erlang, function, arity, native_function, capability)?;
    }
    Ok(())
}

/// erlang:group_leader/0 — returns the calling process's group leader PID.
pub fn bif_group_leader_0(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if !args.is_empty() {
        return Err(badarg());
    }
    context.attached_group_leader().or_else(|_| {
        let pid = context.pid().ok_or_else(badarg)?;
        let facility = context.group_leader_facility().ok_or_else(badarg)?;
        facility.group_leader(pid).ok_or_else(badarg)
    })
}

/// erlang:group_leader/2 — sets `NewLeader` as the group leader for `Pid`.
pub fn bif_group_leader_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [new_leader, pid_term] = args else {
        return Err(badarg());
    };
    if new_leader.as_pid().is_none() {
        return Err(badarg());
    }
    let target_pid = pid_term.as_pid().ok_or_else(badarg)?;

    if context.set_attached_group_leader(target_pid, *new_leader) {
        return Ok(Term::atom(Atom::TRUE));
    }

    let facility = context.group_leader_facility().ok_or_else(badarg)?;
    if facility.set_group_leader(target_pid, *new_leader) {
        Ok(Term::atom(Atom::TRUE))
    } else {
        Err(badarg())
    }
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::native::ProcessContext;
    use crate::process::Process;

    #[test]
    fn group_leader_0_returns_attached_process_group_leader() {
        let mut process = Process::new(7, 16);
        process.set_group_leader(Term::pid(3));
        let mut context = ProcessContext::new();
        context.attach_process(&mut process, 0);

        assert_eq!(bif_group_leader_0(&[], &mut context), Ok(Term::pid(3)));
    }

    #[test]
    fn group_leader_2_sets_attached_process_group_leader() {
        let mut process = Process::new(7, 16);
        let mut context = ProcessContext::new();
        context.attach_process(&mut process, 0);

        assert_eq!(
            bif_group_leader_2(&[Term::pid(3), Term::pid(7)], &mut context),
            Ok(Term::atom(Atom::TRUE))
        );
        assert_eq!(context.attached_group_leader(), Ok(Term::pid(3)));
    }

    #[test]
    fn group_leader_2_sets_facility_target_group_leader() {
        let (facility, mut context) = group_leader_ctx(7, &[(9, Term::pid(9))]);

        assert_eq!(
            bif_group_leader_2(&[Term::pid(3), Term::pid(9)], &mut context),
            Ok(Term::atom(Atom::TRUE))
        );
        assert_eq!(facility.group_leader(9), Some(Term::pid(3)));
    }

    #[test]
    fn group_leader_0_can_read_from_facility_without_attached_process() {
        let (_facility, mut context) = group_leader_ctx(7, &[(7, Term::pid(4))]);

        assert_eq!(bif_group_leader_0(&[], &mut context), Ok(Term::pid(4)));
    }

    #[test]
    fn group_leader_2_rejects_missing_target_pid() {
        let (_facility, mut context) = group_leader_ctx(7, &[]);

        assert_eq!(
            bif_group_leader_2(&[Term::pid(3), Term::pid(99)], &mut context),
            Err(Term::atom(Atom::BADARG))
        );
    }

    #[test]
    fn group_leader_bifs_reject_wrong_arity_and_non_pid_arguments() {
        let (_facility, mut context) = group_leader_ctx(7, &[(7, Term::pid(7))]);

        assert_eq!(
            bif_group_leader_0(&[Term::pid(7)], &mut context),
            Err(Term::atom(Atom::BADARG))
        );
        assert_eq!(
            bif_group_leader_2(&[Term::atom(Atom::OK), Term::pid(7)], &mut context),
            Err(Term::atom(Atom::BADARG))
        );
        assert_eq!(
            bif_group_leader_2(&[Term::pid(3), Term::atom(Atom::OK)], &mut context),
            Err(Term::atom(Atom::BADARG))
        );
        assert_eq!(
            bif_group_leader_2(&[Term::pid(3)], &mut context),
            Err(Term::atom(Atom::BADARG))
        );
    }

    #[test]
    fn process_info_bifs_register_group_leader_arities() {
        let atom_table = AtomTable::new();
        let registry = BifRegistryImpl::new();

        register_process_info_bifs(&registry, &atom_table)
            .unwrap_or_else(|error| panic!("process info BIF registration succeeds: {error}"));

        let erlang = atom_table.intern("erlang");
        let group_leader = atom_table.intern("group_leader");
        assert!(registry.lookup(erlang, group_leader, 0).is_some());
        assert!(registry.lookup(erlang, group_leader, 2).is_some());
    }

    fn group_leader_ctx(
        caller_pid: u64,
        entries: &[(u64, Term)],
    ) -> (Arc<MockGroupLeaderFacility>, ProcessContext<'static>) {
        let facility = Arc::new(MockGroupLeaderFacility::new(entries));
        let mut context = ProcessContext::new();
        context.set_pid(Some(caller_pid));
        context.set_group_leader_facility(Some(facility.clone()));
        (facility, context)
    }

    struct MockGroupLeaderFacility {
        leaders: Mutex<HashMap<u64, Term>>,
    }

    impl MockGroupLeaderFacility {
        fn new(entries: &[(u64, Term)]) -> Self {
            Self {
                leaders: Mutex::new(entries.iter().copied().collect()),
            }
        }
    }

    impl GroupLeaderFacility for MockGroupLeaderFacility {
        fn group_leader(&self, pid: u64) -> Option<Term> {
            self.leaders
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(&pid)
                .copied()
        }

        fn set_group_leader(&self, pid: u64, group_leader: Term) -> bool {
            let mut leaders = self
                .leaders
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(leader) = leaders.get_mut(&pid) else {
                return false;
            };
            *leader = group_leader;
            true
        }
    }
}
