//! Erlang Term Storage registry, metadata, and lifecycle support.

pub mod bag;
pub mod copy;
pub mod match_arena;
pub mod match_spec;
pub mod ordered_set;
pub(crate) mod owned_key;
pub mod set;
pub mod table;
pub mod term_key;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::{DashMap, mapref::entry::Entry};

use crate::atom::{Atom, AtomTable};
use crate::term::Term;
use crate::term::boxed::Tuple;

pub use bag::{EtsBag, EtsDuplicateBag};
pub use copy::{OwnedTerm, copy_term_to_ets, copy_term_to_heap};
pub use match_arena::MatchArena;
pub use match_spec::{CompiledMatchSpec, MatchSpec, MatchSpecError};
pub use ordered_set::EtsOrderedSet;
pub use set::EtsSet;
pub use table::{
    AccessOp, EtsError, EtsHeir, EtsOwner, EtsTable, EtsTableId, EtsTableMetadata, EtsTableType,
    Protection,
};
pub use term_key::TermKey;

pub(crate) fn tuple_key(tuple_term: Term, keypos: usize) -> Result<Term, EtsError> {
    let tuple = Tuple::new(tuple_term).ok_or(EtsError::Badarg)?;
    let key_index = keypos.checked_sub(1).ok_or(EtsError::Badarg)?;
    tuple.get(key_index).ok_or(EtsError::Badarg)
}

/// Concurrent ETS table registry shared by schedulers.
pub struct EtsRegistry {
    next_table_id: AtomicU64,
    tables: DashMap<EtsTableId, Arc<dyn EtsTable>>,
    names: DashMap<Atom, EtsTableId>,
    atom_table: Arc<AtomTable>,
}

impl EtsRegistry {
    /// Construct a registry bound to the VM atom table.
    ///
    /// The handle is not decoration: `ordered_set` orders atom keys by atom
    /// *name*, which requires resolving each atom against the same table that
    /// interned it. A registry given any other table silently degrades atom
    /// ordering to raw intern-index order.
    #[must_use]
    pub fn new(atom_table: Arc<AtomTable>) -> Self {
        Self {
            next_table_id: AtomicU64::new(1),
            tables: DashMap::new(),
            names: DashMap::new(),
            atom_table,
        }
    }

    pub fn create_table(&self, mut metadata: EtsTableMetadata) -> EtsTableId {
        if metadata.id == 0 {
            metadata.id = self.allocate_table_id();
        } else {
            self.reserve_table_id(metadata.id);
        }
        let id = metadata.id;
        let name = metadata.name;
        let table = self.table_from_metadata(metadata);
        if let Some(previous_table) = self.tables.insert(id, table)
            && let Some(previous_name) = previous_table.metadata().name
        {
            self.names
                .remove_if(&previous_name, |_, table_id| *table_id == id);
        }
        if let Some(name) = name {
            self.names.insert(name, id);
        }
        id
    }

    /// Create a table while rejecting duplicate named-table bindings.
    ///
    /// This is the ETS BIF creation path: Erlang `ets:new(Name,
    /// [named_table])` must fail with `badarg` when `Name` is already bound.
    /// The name reservation is performed with a `DashMap` entry guard so two
    /// concurrent named creates cannot both succeed.
    pub fn try_create_table(&self, mut metadata: EtsTableMetadata) -> Result<EtsTableId, EtsError> {
        if metadata.id == 0 {
            metadata.id = self.allocate_table_id();
        } else {
            self.reserve_table_id(metadata.id);
        }
        let id = metadata.id;
        let name = metadata.name;
        let table = self.table_from_metadata(metadata);

        let Some(name) = name else {
            self.tables.insert(id, table);
            return Ok(id);
        };

        match self.names.entry(name) {
            Entry::Occupied(_existing) => Err(EtsError::Badarg),
            Entry::Vacant(entry) => {
                self.tables.insert(id, table);
                entry.insert(id);
                Ok(id)
            }
        }
    }

    fn table_from_metadata(&self, metadata: EtsTableMetadata) -> Arc<dyn EtsTable> {
        match metadata.table_type {
            EtsTableType::Set => Arc::new(EtsSet::new(metadata)),
            EtsTableType::OrderedSet => Arc::new(EtsOrderedSet::with_atom_table(
                metadata,
                Arc::clone(&self.atom_table),
            )),
            EtsTableType::Bag => Arc::new(EtsBag::new(metadata)),
            EtsTableType::DuplicateBag => Arc::new(EtsDuplicateBag::new(metadata)),
        }
    }

    fn allocate_table_id(&self) -> EtsTableId {
        self.next_table_id.fetch_add(1, Ordering::Relaxed)
    }

    fn reserve_table_id(&self, id: EtsTableId) {
        let mut current = self.next_table_id.load(Ordering::Relaxed);
        while current <= id {
            match self.next_table_id.compare_exchange_weak(
                current,
                id.saturating_add(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(next) => current = next,
            }
        }
    }

    #[must_use]
    pub fn lookup_table(&self, id: EtsTableId) -> Option<Arc<dyn EtsTable>> {
        self.tables.get(&id).map(|entry| Arc::clone(entry.value()))
    }

    #[must_use]
    pub fn lookup_named_table(&self, name: Atom) -> Option<Arc<dyn EtsTable>> {
        let id = *self.names.get(&name)?;
        self.lookup_table(id)
    }

    pub fn delete_table(&self, id: EtsTableId) -> bool {
        let Some(table) = self.tables.remove(&id).map(|(_, v)| v) else {
            return false;
        };
        if let Some(name) = table.metadata().name {
            self.names.remove_if(&name, |_, table_id| *table_id == id);
        }
        true
    }

    pub fn delete_tables_owned_by(&self, owner_pid: u64) {
        let owned_ids: Vec<EtsTableId> = self
            .tables
            .iter()
            .filter(|entry| entry.value().metadata().owner.get() == owner_pid)
            .map(|entry| *entry.key())
            .collect();
        for id in owned_ids {
            self.delete_table(id);
        }
    }

    #[must_use]
    pub fn table_ids_owned_by(&self, owner_pid: u64) -> Vec<EtsTableId> {
        self.tables
            .iter()
            .filter(|entry| entry.value().metadata().owner.get() == owner_pid)
            .map(|entry| *entry.key())
            .collect()
    }

    pub fn transfer_table_owner(&self, table_id: EtsTableId, new_owner: u64) -> bool {
        let Some(table) = self.lookup_table(table_id) else {
            return false;
        };
        table.transfer_owner(new_owner);
        true
    }

    #[must_use]
    pub fn lookup_table_by_name(&self, name: Atom) -> Option<EtsTableId> {
        self.names.get(&name).map(|entry| *entry.value())
    }

    #[must_use]
    pub fn table_count(&self) -> usize {
        self.tables.len()
    }
}

// `EtsRegistry` deliberately has no `Default`: a default-constructed registry
// would have no VM atom table, and the only way to satisfy `Default` would be
// to invent a private one — which is precisely the defect this construction
// exists to prevent. `clippy::new_without_default` does not apply now that
// `new` takes the table as a parameter.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atom::Atom;
    use crate::term::{Term, boxed};

    fn metadata(table_type: EtsTableType) -> EtsTableMetadata {
        EtsTableMetadata::new(Some(Atom::OK), 0, table_type, Protection::Protected, 7)
    }

    fn test_atom_table() -> Arc<AtomTable> {
        Arc::new(AtomTable::with_common_atoms())
    }

    #[test]
    fn registry_creates_set_table_and_round_trips_through_trait_object() {
        let registry = EtsRegistry::new(test_atom_table());
        let table_id = registry.create_table(metadata(EtsTableType::Set));
        let table = registry.lookup_table(table_id).expect("set table exists");

        let mut tuple_heap = [0_u64; 3];
        let tuple =
            boxed::write_tuple(&mut tuple_heap, &[Term::atom(Atom::OK), Term::small_int(1)])
                .expect("tuple fits");

        table.insert(tuple).expect("tuple inserts");
        let rows = table.lookup(Term::atom(Atom::OK));
        assert_eq!(rows.len(), 1);
        assert!(crate::term::compare::exact_eq(rows[0].root(), tuple));
    }

    /// An `ordered_set` created through the registry must order atom keys by
    /// their **names**, using the VM atom table the registry was built with.
    ///
    /// The two names here are deliberately outside `COMMON_ATOMS`: two common
    /// atoms resolve identically in any table seeded by
    /// `AtomTable::with_common_atoms`, so a fixture built from them would pass
    /// even when the table is comparing against a private table it built for
    /// itself. Interning in reverse lexical order ("zebra" before "apple")
    /// makes raw intern-index order the exact opposite of name order, so the
    /// two orderings are distinguishable.
    #[test]
    fn registry_ordered_set_sorts_atom_keys_by_name_from_the_vm_atom_table() {
        let atom_table = test_atom_table();
        let zebra = atom_table.intern("zebra");
        let apple = atom_table.intern("apple");
        assert!(
            zebra.index() < apple.index(),
            "fixture precondition: reverse lexical intern order"
        );

        let registry = EtsRegistry::new(Arc::clone(&atom_table));
        let table_id = registry.create_table(metadata(EtsTableType::OrderedSet));
        let table = registry
            .lookup_table(table_id)
            .expect("ordered_set table exists");

        let mut zebra_heap = [0_u64; 3];
        let zebra_row =
            boxed::write_tuple(&mut zebra_heap, &[Term::atom(zebra), Term::small_int(1)])
                .expect("tuple fits");
        let mut apple_heap = [0_u64; 3];
        let apple_row =
            boxed::write_tuple(&mut apple_heap, &[Term::atom(apple), Term::small_int(2)])
                .expect("tuple fits");

        table.insert(zebra_row).expect("zebra row inserts");
        table.insert(apple_row).expect("apple row inserts");

        let rows = table.tab2list();
        assert_eq!(rows.len(), 2);

        let first_key = tuple_key(rows[0].root(), 1).expect("stored row has a key");
        let first_name = first_key
            .as_atom()
            .and_then(|atom| atom_table.resolve(atom))
            .expect("first key is an atom known to the VM atom table");
        let second_key = tuple_key(rows[1].root(), 1).expect("stored row has a key");
        let second_name = second_key
            .as_atom()
            .and_then(|atom| atom_table.resolve(atom))
            .expect("second key is an atom known to the VM atom table");

        assert_eq!(
            (first_name, second_name),
            ("apple", "zebra"),
            "ordered_set must sort atom keys by name via the VM atom table"
        );
    }

    #[test]
    fn registry_does_not_reuse_explicit_table_ids_for_implicit_tables() {
        let registry = EtsRegistry::new(test_atom_table());
        let mut explicit = metadata(EtsTableType::Set);
        explicit.id = 7;

        assert_eq!(registry.create_table(explicit), 7);

        let implicit_id = registry.create_table(EtsTableMetadata {
            name: None,
            ..metadata(EtsTableType::Set)
        });

        assert_ne!(implicit_id, 7);
        assert!(implicit_id > 7);
        assert!(registry.lookup_table(7).is_some());
        assert!(registry.lookup_table(implicit_id).is_some());
    }

    #[test]
    fn registry_keeps_reused_names_bound_to_latest_table_when_old_table_deleted() {
        let registry = EtsRegistry::new(test_atom_table());
        let first_id = registry.create_table(metadata(EtsTableType::Set));
        let second_id = registry.create_table(metadata(EtsTableType::Set));

        assert_ne!(first_id, second_id);
        assert_eq!(
            registry
                .lookup_named_table(Atom::OK)
                .expect("latest name binding exists")
                .metadata()
                .id,
            second_id
        );

        assert!(registry.delete_table(first_id));
        assert_eq!(
            registry
                .lookup_named_table(Atom::OK)
                .expect("newer name binding survives old table deletion")
                .metadata()
                .id,
            second_id
        );
    }

    #[test]
    fn try_create_table_rejects_duplicate_names_without_rebinding() {
        let registry = EtsRegistry::new(test_atom_table());
        let first_id = registry
            .try_create_table(metadata(EtsTableType::Set))
            .expect("first named create succeeds");

        assert_eq!(
            registry.try_create_table(metadata(EtsTableType::Bag)),
            Err(EtsError::Badarg)
        );
        assert_eq!(registry.lookup_table_by_name(Atom::OK), Some(first_id));
        assert_eq!(registry.table_count(), 1);
    }
}
