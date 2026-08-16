//! Process group registry and `pg` module BIFs.
//!
//! Beamr keeps pg membership in a scheduler-owned registry so local process
//! exits and distribution lifecycle events can remove stale members without
//! depending on per-process dictionaries.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

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
    /// Swappable propagation backend.
    ///
    /// Held behind an `RwLock` so the real `SchedulerPgPropagation` can be
    /// installed via [`PgRegistry::set_propagation`] *after* `SharedState`
    /// exists. `PgRegistry` is itself a field of `SharedState`, so the
    /// propagation cannot be supplied at construction without an `Arc` cycle;
    /// the registry is built with a `NullPgPropagation` and the real backend
    /// (holding a `Weak<SharedState>`) is swapped in once `SharedState` is
    /// constructed.
    propagation: RwLock<Arc<dyn PgPropagation>>,
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
            propagation: RwLock::new(propagation),
        }
    }

    /// Replace the propagation backend.
    ///
    /// Used by the scheduler to install the real `SchedulerPgPropagation` once
    /// `SharedState` exists, resolving the construction-order/`Arc`-cycle
    /// problem (see the `PgRegistry::propagation` field documentation).
    pub fn set_propagation(&self, propagation: Arc<dyn PgPropagation>) {
        *self
            .propagation
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = propagation;
    }

    /// Snapshot the current propagation backend, releasing the lock before the
    /// caller broadcasts so a blocking send never runs under the `RwLock`.
    fn propagation(&self) -> Arc<dyn PgPropagation> {
        Arc::clone(
            &self
                .propagation
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
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
            // Broadcast outside the PgState lock (already dropped above) and
            // with the propagation RwLock released — `propagation()` snapshots
            // the backend so a blocking send never runs under either lock.
            self.propagation()
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
            self.propagation()
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
        let mut members: Vec<_> = self
            .lock_state()
            .groups
            .get(&(scope, group))
            .map(|members| members.remote.iter().copied().collect())
            .unwrap_or_default();
        members.sort_by_key(|member| (member.node.index(), member.pid_number, member.serial));
        members
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

    /// Remove a local process from every scope/group locally, returning the
    /// `Leave` updates for each group it was actually in.
    ///
    /// This performs the synchronous local purge only — it does **not**
    /// broadcast. It holds the `PgState` lock solely for the in-memory mutation
    /// and returns after dropping the guard, so it is safe to call on a latency-
    /// sensitive path (such as process exit). The caller is responsible for
    /// propagating the returned updates.
    pub fn remove_pid_from_all_scopes_local(&self, pid: u64) -> Vec<PgUpdate> {
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
        drop(state);
        updates
    }

    /// Remove a local process from every scope/group, broadcasting each actual leave.
    pub fn remove_pid_from_all_scopes(&self, pid: u64) {
        let updates = self.remove_pid_from_all_scopes_local(pid);
        let propagation = self.propagation();
        for update in updates {
            propagation.broadcast(update);
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

pub(crate) fn bif_start_link_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [scope] = args else {
        return Err(badarg());
    };
    let scope = scope.as_atom().ok_or_else(badarg)?;
    context.pg_facility().ok_or_else(badarg)?.start_scope(scope);
    Ok(Term::atom(Atom::OK))
}

pub(crate) fn bif_join_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [group, pid] = args else {
        return Err(badarg());
    };
    let facility = context.pg_facility().ok_or_else(badarg)?;
    join(facility, facility.default_scope(), *group, *pid)
}

pub(crate) fn bif_join_3(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [scope, group, pid] = args else {
        return Err(badarg());
    };
    let scope = scope.as_atom().ok_or_else(badarg)?;
    let facility = context.pg_facility().ok_or_else(badarg)?;
    join(facility, scope, *group, *pid)
}

pub(crate) fn bif_leave_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [group, pid] = args else {
        return Err(badarg());
    };
    let facility = context.pg_facility().ok_or_else(badarg)?;
    leave(facility, facility.default_scope(), *group, *pid)
}

pub(crate) fn bif_leave_3(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [scope, group, pid] = args else {
        return Err(badarg());
    };
    let scope = scope.as_atom().ok_or_else(badarg)?;
    let facility = context.pg_facility().ok_or_else(badarg)?;
    leave(facility, scope, *group, *pid)
}

pub(crate) fn bif_get_members_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [group] = args else {
        return Err(badarg());
    };
    let default_scope = context.pg_facility().ok_or_else(badarg)?.default_scope();
    members(context, default_scope, *group, true)
}

pub(crate) fn bif_get_members_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [scope, group] = args else {
        return Err(badarg());
    };
    let scope = scope.as_atom().ok_or_else(badarg)?;
    members(context, scope, *group, true)
}

pub(crate) fn bif_get_local_members_1(
    args: &[Term],
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    let [group] = args else {
        return Err(badarg());
    };
    let default_scope = context.pg_facility().ok_or_else(badarg)?.default_scope();
    members(context, default_scope, *group, false)
}

pub(crate) fn bif_get_local_members_2(
    args: &[Term],
    context: &mut ProcessContext,
) -> Result<Term, Term> {
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
mod ar1_row4_site2_tests {
    // ⛔ DEFECT-ASSERTING TESTS — READ THIS BEFORE TRUSTING A GREEN.
    //
    // These pin the MEASURED CORRUPT SURFACE of AR-1 row 4 at f993280. They do
    // NOT assert correct behaviour, so a green here means "the defect is still
    // present, exactly as measured" — never "this site is safe".
    //
    // ⇒ THEY GO RED WHEN AR-1 IS FIXED, AND THAT IS THE POINT. The fix lane
    // INVERTS them to assert correctness rather than deleting them; the pinned
    // counts below are the surface the fix has to move.

    use std::sync::Arc;

    use super::{PgFacility, RemoteMember, members};
    use crate::atom::{Atom, AtomTable};
    use crate::native::ProcessContext;
    use crate::process::Process;
    use crate::term::Term;
    use crate::term::boxed::{Cons, ExternalPid};

    const HEAP: usize = 512;
    const SERIAL: u64 = 5;

    /// Facility stub returning a fixed roster. Only the two member accessors do
    /// anything; the rest are inert, because this probe drives exactly one
    /// function and a stub that answers unasked questions can drift silently.
    struct RosterFacility {
        scope: Atom,
        local: Vec<u64>,
        remote: Vec<RemoteMember>,
    }

    impl PgFacility for RosterFacility {
        fn default_scope(&self) -> Atom {
            self.scope
        }
        fn start_scope(&self, _scope: Atom) {}
        fn join(&self, _scope: Atom, _group: Atom, _pid: u64) {}
        fn leave(&self, _scope: Atom, _group: Atom, _pid: u64) {}
        fn local_members(&self, _scope: Atom, _group: Atom) -> Vec<u64> {
            self.local.clone()
        }
        fn remote_members(&self, _scope: Atom, _group: Atom) -> Vec<RemoteMember> {
            self.remote.clone()
        }
    }

    /// One cell. `remote` selects the arm: true = remote members (allocating),
    /// false = local members (immediates, the structural control).
    fn members_round_trip(count: usize, remote: bool) -> Result<(), String> {
        let table = Arc::new(AtomTable::with_common_atoms());
        let scope = table.intern("ar1_site2_scope");
        let group_atom = table.intern("ar1_site2_group");
        let node = table.intern("ar1_site2_node@host");

        let facility = RosterFacility {
            scope,
            local: if remote {
                Vec::new()
            } else {
                // Pid numbers start at 1: 0 may be reserved, and a probe whose
                // control arm quietly refused would be indistinguishable from one
                // that passed.
                (1..=count as u64).collect()
            },
            remote: if remote {
                (1..=count as u64)
                    .map(|pid_number| RemoteMember {
                        node,
                        pid_number,
                        serial: SERIAL,
                    })
                    .collect()
            } else {
                Vec::new()
            },
        };

        let mut process = Process::new(2, HEAP);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&table)));
        context.attach_process(&mut process, 0);
        context.set_pg_facility(Some(Arc::new(facility)));

        let list = members(&mut context, scope, Term::atom(group_atom), true)
            .map_err(|_| "members returned an error term".to_string())?;

        // Iterative, hard-capped: a stale carrier can alias a cons into a cycle.
        let cap = count * 2 + 16;
        let mut seen = 0usize;
        let mut tail = list;
        while !tail.is_nil() {
            if seen > cap {
                return Err(format!(
                    "list did not terminate within {cap} cells — cyclic tail, carrier `terms` went stale"
                ));
            }
            let cons = Cons::new(tail).ok_or_else(|| {
                format!("entry {seen}: tail is not a cons — carrier `terms` went stale")
            })?;
            let want = seen as u64 + 1;
            if remote {
                let external = ExternalPid::new(cons.head()).ok_or_else(|| {
                    format!(
                        "entry {seen}: head is not an external pid — carrier `terms` went stale"
                    )
                })?;
                if external.node() != Some(node) {
                    return Err(format!(
                        "entry {seen}: node atom differs — carrier `terms` went stale"
                    ));
                }
                if external.pid_number() != want {
                    return Err(format!(
                        "entry {seen}: pid_number {} != {want} — carrier `terms` went stale",
                        external.pid_number()
                    ));
                }
                if external.serial() != SERIAL {
                    return Err(format!(
                        "entry {seen}: serial {} != {SERIAL} — carrier `terms` went stale",
                        external.serial()
                    ));
                }
            } else {
                let expected = Term::try_pid(want).ok_or_else(|| "pid does not fit".to_string())?;
                if cons.head() != expected {
                    return Err(format!(
                        "entry {seen}: local pid differs — carrier `terms` went stale"
                    ));
                }
            }
            seen += 1;
            tail = cons.tail();
        }
        if seen != count {
            return Err(format!("recovered {seen} members, put {count}"));
        }
        Ok(())
    }

    #[test]
    fn ar1_site2_members_band() {
        // Spans the ~128-member collection point (512 words / 4 per member) in
        // both directions, and continues well past it.
        //
        // ⭐ THE DENSE RUN FROM 129 TO 199 IS DELIBERATE AND WAS ADDED AFTER A
        // FIRST PASS. The coarse sweep found exactly ONE red cell, at 150,
        // sitting between a clean region ending at 128 and a refusal region
        // starting at 200 — so the site's entire OBSERVABLE band is the gap
        // between "the heap must now collect" and "the allocator refuses
        // outright". A single red cell is a demonstration but not a located
        // edge, and the width of that gap is the interesting quantity here.
        const COUNTS: &[usize] = &[
            1, 10, 50, 100, 120, 128, // below: no collection needed
            129, 130, 132, 135, 140, 145, 150, 160, 170, 180, 190, 199, // the live band
            200, 300, 500, // above: allocator refuses, NOT evidence
        ];

        let mut remote_cells = Vec::new();
        let mut local_cells = Vec::new();
        for &count in COUNTS {
            for arm_remote in [true, false] {
                let verdict = match members_round_trip(count, arm_remote) {
                    Ok(()) => "ok".to_string(),
                    Err(reason) => reason,
                };
                eprintln!(
                    "site 2 arm {} count {count:>4} : {verdict}",
                    if arm_remote { "REMOTE" } else { "LOCAL " }
                );
                if arm_remote {
                    remote_cells.push((count, verdict));
                } else {
                    local_cells.push((count, verdict));
                }
            }
        }

        let is_red = |v: &String| v != "ok" && !v.contains("returned an error term");
        let remote_red = remote_cells.iter().filter(|(_, v)| is_red(v)).count();
        let remote_ok = remote_cells.iter().filter(|(_, v)| v == "ok").count();
        let local_red: Vec<_> = local_cells.iter().filter(|(_, v)| is_red(v)).collect();
        let local_ok = local_cells.iter().filter(|(_, v)| v == "ok").count();

        eprintln!("site 2 arm REMOTE: {remote_red} red, {remote_ok} clean");
        eprintln!(
            "site 2 arm LOCAL : {} red, {local_ok} clean",
            local_red.len()
        );

        // ⛔ COVERAGE — site 1's lesson. The sweep must demand more heap than
        // exists, or its clean cells describe the knob's range and not the site.
        let largest = COUNTS.iter().copied().max().unwrap_or(0);
        assert!(
            largest * 4 > HEAP,
            "INSTRUMENT NOT SHOWN AWAKE: the largest count {largest} demands {} words against a \
             {HEAP}-word heap, so no collection is forced anywhere in this sweep and every cell \
             is vacuous.",
            largest * 4
        );

        assert!(
            remote_ok > 0,
            "control: some REMOTE cell must be clean, or the reader is broken rather than the \
             site defective"
        );

        // ⛔ THE STRUCTURAL NEGATIVE CONTROL. Local members are immediates, so
        // the accumulation loop allocates nothing and cannot collect.
        assert!(
            local_red.is_empty(),
            "ATTRIBUTION BROKEN: arm LOCAL corrupted {} cells, but `Term::try_pid` is an immediate \
             and that loop allocates nothing. The exposure is not `alloc_external_pid` — \
             re-derive it.\n{local_red:#?}",
            local_red.len()
        );

        assert!(
            remote_red > 0,
            "site 2: no REMOTE cell corrupted the carrier. Under the site-14 law that is \
             UNRESOLVED, not defended.\n{remote_cells:#?}"
        );

        // Waffles' condition 3 — the whole surface pinned, so a passing run
        // carries its evidence instead of only the absence of a failure.
        assert_eq!(
            (remote_red, remote_ok, local_red.len(), local_ok),
            (9, 6, 0, 21),
            "site 2 surface drifted from the measured band.\nREMOTE: {remote_cells:#?}\nLOCAL: {local_cells:#?}"
        );
    }
}
