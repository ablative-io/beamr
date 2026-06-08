//! Process group registry and `pg` module BIFs.
//!
//! Beamr keeps pg membership in a scheduler-owned registry so local process
//! exits and distribution lifecycle events can remove stale members without
//! depending on per-process dictionaries.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::atom::{Atom, AtomTable};
use crate::native::{
    BifRegistryImpl, Capability, NativeFn, NativeRegistrationError, ProcessContext,
};
use crate::term::Term;

const DEFAULT_SCOPE_NAME: &str = "pg";

type Scope = Atom;
type Group = Atom;

/// Stable identity for a remote member advertised by another node.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct RemoteMember {
    /// Remote node atom.
    pub node: Atom,
    /// Remote PID number on that node.
    pub pid_number: u64,
    /// Remote PID serial.
    pub serial: u64,
}

/// A pg membership update suitable for transport-independent propagation.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PgUpdate {
    /// Local process joined a scope/group.
    Join {
        /// Scope atom.
        scope: Atom,
        /// Group atom.
        group: Atom,
        /// Local PID number.
        pid: u64,
    },
    /// Local process left a scope/group.
    Leave {
        /// Scope atom.
        scope: Atom,
        /// Group atom.
        group: Atom,
        /// Local PID number.
        pid: u64,
    },
}

/// Transport abstraction used by PgRegistry to broadcast local membership changes.
pub trait PgPropagation: Send + Sync {
    /// Broadcast an update to connected nodes.
    fn broadcast(&self, update: PgUpdate);
}

#[derive(Default)]
struct NullPgPropagation;

impl PgPropagation for NullPgPropagation {
    fn broadcast(&self, _update: PgUpdate) {}
}

#[derive(Default)]
struct GroupMembers {
    local: BTreeSet<u64>,
    remote: HashSet<RemoteMember>,
}

#[derive(Default)]
struct PgState {
    scopes: HashSet<Scope>,
    groups: HashMap<(Scope, Group), GroupMembers>,
}

/// Scheduler-owned pg registry.
pub struct PgRegistry {
    default_scope: Scope,
    state: Mutex<PgState>,
    propagation: Arc<dyn PgPropagation>,
}

impl PgRegistry {
    /// Create a registry with the default `pg` scope interned in `atom_table`.
    #[must_use]
    pub fn new(atom_table: &AtomTable) -> Self {
        Self::with_propagation(atom_table, Arc::new(NullPgPropagation))
    }

    /// Create a registry using an explicit propagation backend.
    #[must_use]
    pub fn with_propagation(atom_table: &AtomTable, propagation: Arc<dyn PgPropagation>) -> Self {
        let default_scope = atom_table.intern(DEFAULT_SCOPE_NAME);
        let mut scopes = HashSet::new();
        scopes.insert(default_scope);
        Self {
            default_scope,
            state: Mutex::new(PgState {
                scopes,
                groups: HashMap::new(),
            }),
            propagation,
        }
    }

    /// Return the default pg scope atom.
    #[must_use]
    pub const fn default_scope(&self) -> Atom {
        self.default_scope
    }

    /// Create a scope if it does not already exist.
    pub fn start_scope(&self, scope: Scope) {
        self.lock_state().scopes.insert(scope);
    }

    /// Add a local PID to a group in the supplied scope. Duplicate joins are idempotent.
    pub fn join(&self, scope: Scope, group: Group, pid: u64) {
        let inserted = {
            let mut state = self.lock_state();
            state.scopes.insert(scope);
            state
                .groups
                .entry((scope, group))
                .or_default()
                .local
                .insert(pid)
        };
        if inserted {
            self.propagation
                .broadcast(PgUpdate::Join { scope, group, pid });
        }
    }

    /// Remove a local PID from a group in the supplied scope.
    pub fn leave(&self, scope: Scope, group: Group, pid: u64) {
        let removed = {
            let mut state = self.lock_state();
            match state.groups.get_mut(&(scope, group)) {
                Some(members) => members.local.remove(&pid),
                None => false,
            }
        };
        if removed {
            self.propagation
                .broadcast(PgUpdate::Leave { scope, group, pid });
        }
    }

    /// Return local members for a scope/group.
    #[must_use]
    pub fn local_members(&self, scope: Scope, group: Group) -> Vec<u64> {
        self.lock_state()
            .groups
            .get(&(scope, group))
            .map(|members| members.local.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Return remote members for a scope/group.
    #[must_use]
    pub fn remote_members(&self, scope: Scope, group: Group) -> Vec<RemoteMember> {
        self.lock_state()
            .groups
            .get(&(scope, group))
            .map(|members| members.remote.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Apply a join received from a remote node.
    pub fn apply_remote_join(
        &self,
        scope: Scope,
        group: Group,
        node: Atom,
        pid_number: u64,
        serial: u64,
    ) {
        let mut state = self.lock_state();
        state.scopes.insert(scope);
        state
            .groups
            .entry((scope, group))
            .or_default()
            .remote
            .insert(RemoteMember {
                node,
                pid_number,
                serial,
            });
    }

    /// Apply a leave received from a remote node.
    pub fn apply_remote_leave(
        &self,
        scope: Scope,
        group: Group,
        node: Atom,
        pid_number: u64,
        serial: u64,
    ) {
        if let Some(members) = self.lock_state().groups.get_mut(&(scope, group)) {
            members.remote.remove(&RemoteMember {
                node,
                pid_number,
                serial,
            });
        }
    }

    /// Remove a local process from every scope/group, broadcasting each actual leave.
    pub fn remove_pid_from_all_scopes(&self, pid: u64) {
        let updates = {
            let mut state = self.lock_state();
            let mut updates = Vec::new();
            for ((scope, group), members) in &mut state.groups {
                if members.local.remove(&pid) {
                    updates.push(PgUpdate::Leave {
                        scope: *scope,
                        group: *group,
                        pid,
                    });
                }
            }
            updates
        };
        for update in updates {
            self.propagation.broadcast(update);
        }
    }

    /// Remove every remote member that belongs to a disconnected node.
    pub fn purge_remote_node(&self, node: Atom) {
        let mut state = self.lock_state();
        for members in state.groups.values_mut() {
            members.remote.retain(|member| member.node != node);
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, PgState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Facility exposed to pg BIFs.
pub trait PgFacility: Send + Sync {
    /// Return the default pg scope atom.
    fn default_scope(&self) -> Atom;
    /// Create a scope if necessary.
    fn start_scope(&self, scope: Atom);
    /// Join a local pid to a scoped group.
    fn join(&self, scope: Atom, group: Atom, pid: u64);
    /// Leave a local pid from a scoped group.
    fn leave(&self, scope: Atom, group: Atom, pid: u64);
    /// Return local member pid numbers.
    fn local_members(&self, scope: Atom, group: Atom) -> Vec<u64>;
    /// Return remote member identities.
    fn remote_members(&self, scope: Atom, group: Atom) -> Vec<RemoteMember>;
}

impl PgFacility for PgRegistry {
    fn default_scope(&self) -> Atom {
        self.default_scope()
    }

    fn start_scope(&self, scope: Atom) {
        self.start_scope(scope);
    }

    fn join(&self, scope: Atom, group: Atom, pid: u64) {
        self.join(scope, group, pid);
    }

    fn leave(&self, scope: Atom, group: Atom, pid: u64) {
        self.leave(scope, group, pid);
    }

    fn local_members(&self, scope: Atom, group: Atom) -> Vec<u64> {
        self.local_members(scope, group)
    }

    fn remote_members(&self, scope: Atom, group: Atom) -> Vec<RemoteMember> {
        self.remote_members(scope, group)
    }
}

type PgBif = (&'static str, u8, NativeFn);

const PG_BIFS: &[PgBif] = &[
    ("start_link", 1, bif_start_link_1),
    ("join", 2, bif_join_2),
    ("join", 3, bif_join_3),
    ("leave", 2, bif_leave_2),
    ("leave", 3, bif_leave_3),
    ("get_members", 1, bif_get_members_1),
    ("get_members", 2, bif_get_members_2),
    ("get_local_members", 1, bif_get_local_members_1),
    ("get_local_members", 2, bif_get_local_members_2),
];

/// Register the `pg` module BIFs.
pub fn register_pg_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let pg = atom_table.intern(DEFAULT_SCOPE_NAME);
    for &(name, arity, function) in PG_BIFS {
        registry.register(
            pg,
            atom_table.intern(name),
            arity,
            function,
            Capability::ProcessLocal,
        )?;
    }
    Ok(())
}

fn bif_start_link_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [scope] = args else {
        return Err(badarg());
    };
    let scope = scope.as_atom().ok_or_else(badarg)?;
    context.pg_facility().ok_or_else(badarg)?.start_scope(scope);
    Ok(Term::atom(Atom::OK))
}

fn bif_join_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [group, pid] = args else {
        return Err(badarg());
    };
    let facility = context.pg_facility().ok_or_else(badarg)?;
    join(facility, facility.default_scope(), *group, *pid)
}

fn bif_join_3(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [scope, group, pid] = args else {
        return Err(badarg());
    };
    let scope = scope.as_atom().ok_or_else(badarg)?;
    let facility = context.pg_facility().ok_or_else(badarg)?;
    join(facility, scope, *group, *pid)
}

fn bif_leave_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [group, pid] = args else {
        return Err(badarg());
    };
    let facility = context.pg_facility().ok_or_else(badarg)?;
    leave(facility, facility.default_scope(), *group, *pid)
}

fn bif_leave_3(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [scope, group, pid] = args else {
        return Err(badarg());
    };
    let scope = scope.as_atom().ok_or_else(badarg)?;
    let facility = context.pg_facility().ok_or_else(badarg)?;
    leave(facility, scope, *group, *pid)
}

fn bif_get_members_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [group] = args else {
        return Err(badarg());
    };
    let default_scope = context.pg_facility().ok_or_else(badarg)?.default_scope();
    members(context, default_scope, *group, true)
}

fn bif_get_members_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [scope, group] = args else {
        return Err(badarg());
    };
    let scope = scope.as_atom().ok_or_else(badarg)?;
    members(context, scope, *group, true)
}

fn bif_get_local_members_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [group] = args else {
        return Err(badarg());
    };
    let default_scope = context.pg_facility().ok_or_else(badarg)?.default_scope();
    members(context, default_scope, *group, false)
}

fn bif_get_local_members_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [scope, group] = args else {
        return Err(badarg());
    };
    let scope = scope.as_atom().ok_or_else(badarg)?;
    members(context, scope, *group, false)
}

fn join(facility: &dyn PgFacility, scope: Atom, group: Term, pid: Term) -> Result<Term, Term> {
    let group = group.as_atom().ok_or_else(badarg)?;
    let pid = pid.as_pid().ok_or_else(badarg)?;
    facility.join(scope, group, pid);
    Ok(Term::atom(Atom::OK))
}

fn leave(facility: &dyn PgFacility, scope: Atom, group: Term, pid: Term) -> Result<Term, Term> {
    let group = group.as_atom().ok_or_else(badarg)?;
    let pid = pid.as_pid().ok_or_else(badarg)?;
    facility.leave(scope, group, pid);
    Ok(Term::atom(Atom::OK))
}

fn members(
    context: &mut ProcessContext,
    scope: Atom,
    group: Term,
    include_remote: bool,
) -> Result<Term, Term> {
    let group = group.as_atom().ok_or_else(badarg)?;
    let (local_members, remote_members) = {
        let facility = context.pg_facility().ok_or_else(badarg)?;
        let remote_members = if include_remote {
            facility.remote_members(scope, group)
        } else {
            Vec::new()
        };
        (facility.local_members(scope, group), remote_members)
    };
    let mut terms = Vec::new();
    for pid in local_members {
        terms.push(Term::try_pid(pid).ok_or_else(badarg)?);
    }
    for remote in remote_members {
        terms.push(context.alloc_external_pid(remote.node, remote.pid_number, remote.serial)?);
    }
    context.alloc_list(&terms)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Process;
    use crate::term::boxed::{Cons, ExternalPid};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingPropagation {
        updates: Mutex<Vec<PgUpdate>>,
    }

    impl PgPropagation for RecordingPropagation {
        fn broadcast(&self, update: PgUpdate) {
            self.updates
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(update);
        }
    }

    fn atom_table() -> AtomTable {
        AtomTable::with_common_atoms()
    }

    fn context<'a>(process: &'a mut Process, registry: Arc<PgRegistry>) -> ProcessContext<'a> {
        let mut context = ProcessContext::new();
        context.attach_process(process, 0);
        let facility: Arc<dyn PgFacility> = registry;
        context.set_pg_facility(Some(facility));
        context
    }

    fn list_terms(mut term: Term) -> Vec<Term> {
        let mut terms = Vec::new();
        while term != Term::NIL {
            let cons = Cons::new(term).expect("proper list cons");
            terms.push(cons.head());
            term = cons.tail();
        }
        terms
    }

    #[test]
    fn join_get_members_leave_round_trips() {
        let atoms = atom_table();
        let registry = Arc::new(PgRegistry::new(&atoms));
        let group = atoms.intern("workers");
        let pid = Term::pid(42);
        let mut process = Process::new(1, 128);
        let mut context = context(&mut process, Arc::clone(&registry));

        assert_eq!(
            bif_join_2(&[Term::atom(group), pid], &mut context),
            Ok(Term::atom(Atom::OK))
        );
        let members = bif_get_members_1(&[Term::atom(group)], &mut context).expect("members");
        assert_eq!(list_terms(members), vec![pid]);

        assert_eq!(
            bif_leave_2(&[Term::atom(group), pid], &mut context),
            Ok(Term::atom(Atom::OK))
        );
        let members = bif_get_members_1(&[Term::atom(group)], &mut context).expect("members");
        assert!(list_terms(members).is_empty());
    }

    #[test]
    fn duplicate_join_does_not_duplicate_members() {
        let atoms = atom_table();
        let registry = PgRegistry::new(&atoms);
        let group = atoms.intern("workers");
        registry.join(registry.default_scope(), group, 7);
        registry.join(registry.default_scope(), group, 7);

        assert_eq!(
            registry.local_members(registry.default_scope(), group),
            vec![7]
        );
    }

    #[test]
    fn process_exit_removes_pid_from_all_groups_and_broadcasts_leaves() {
        let atoms = atom_table();
        let propagation = Arc::new(RecordingPropagation::default());
        let registry = PgRegistry::with_propagation(&atoms, propagation.clone());
        let group_a = atoms.intern("a");
        let group_b = atoms.intern("b");
        registry.join(registry.default_scope(), group_a, 7);
        registry.join(registry.default_scope(), group_b, 7);

        registry.remove_pid_from_all_scopes(7);

        assert!(
            registry
                .local_members(registry.default_scope(), group_a)
                .is_empty()
        );
        assert!(
            registry
                .local_members(registry.default_scope(), group_b)
                .is_empty()
        );
        let updates = propagation
            .updates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(updates.contains(&PgUpdate::Leave {
            scope: registry.default_scope(),
            group: group_a,
            pid: 7
        }));
        assert!(updates.contains(&PgUpdate::Leave {
            scope: registry.default_scope(),
            group: group_b,
            pid: 7
        }));
    }

    #[test]
    fn remote_apply_join_visible_only_in_all_members_and_purged_on_node_down() {
        let atoms = atom_table();
        let registry = Arc::new(PgRegistry::new(&atoms));
        let group = atoms.intern("workers");
        let node = atoms.intern("remote@host");
        registry.apply_remote_join(registry.default_scope(), group, node, 99, 1);

        let mut process = Process::new(1, 128);
        let mut context = context(&mut process, Arc::clone(&registry));
        let local = bif_get_local_members_1(&[Term::atom(group)], &mut context).expect("local");
        assert!(list_terms(local).is_empty());

        let all = bif_get_members_1(&[Term::atom(group)], &mut context).expect("all");
        let all = list_terms(all);
        assert_eq!(all.len(), 1);
        let external = ExternalPid::new(all[0]).expect("external pid");
        assert_eq!(external.node(), Some(node));
        assert_eq!(external.pid_number(), 99);
        assert_eq!(external.serial(), 1);

        registry.purge_remote_node(node);
        let all = bif_get_members_1(&[Term::atom(group)], &mut context).expect("all");
        assert!(list_terms(all).is_empty());
    }

    #[test]
    fn scoped_groups_are_independent() {
        let atoms = atom_table();
        let registry = PgRegistry::new(&atoms);
        let scope_a = atoms.intern("scope_a");
        let scope_b = atoms.intern("scope_b");
        let group = atoms.intern("workers");
        registry.start_scope(scope_a);
        registry.start_scope(scope_b);
        registry.join(scope_a, group, 1);
        registry.join(scope_b, group, 2);

        assert_eq!(registry.local_members(scope_a, group), vec![1]);
        assert_eq!(registry.local_members(scope_b, group), vec![2]);
        assert!(
            registry
                .local_members(registry.default_scope(), group)
                .is_empty()
        );
    }
}
