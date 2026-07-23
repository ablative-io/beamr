//! Hash-based ETS `set` table implementation.

use std::sync::Arc;

use dashmap::{DashMap, mapref::entry::Entry};

use crate::ets::copy::{OwnedTerm, copy_term_to_ets};
use crate::ets::owned_key::OwnedEtsKey;
use crate::ets::{EtsError, EtsTable, EtsTableMetadata};
use crate::term::{Term, boxed::Tuple, compare, hash::EtsKey};

/// ETS `set` table backed by a concurrent hash map.
///
/// Rows are deep-copied into ETS-owned storage on insert; nothing in the map
/// points into any process heap.
pub struct EtsSet {
    metadata: EtsTableMetadata,
    entries: DashMap<OwnedEtsKey, Arc<OwnedTerm>>,
}

impl EtsSet {
    #[must_use]
    pub fn new(metadata: EtsTableMetadata) -> Self {
        Self {
            metadata,
            entries: DashMap::new(),
        }
    }

    fn tuple_key(&self, tuple_term: Term) -> Result<Term, EtsError> {
        let tuple = Tuple::new(tuple_term).ok_or(EtsError::Badarg)?;
        let key_index = self
            .metadata
            .keypos
            .checked_sub(1)
            .ok_or(EtsError::Badarg)?;
        tuple.get(key_index).ok_or(EtsError::Badarg)
    }
}

impl EtsTable for EtsSet {
    fn metadata(&self) -> &EtsTableMetadata {
        &self.metadata
    }

    fn insert(&self, tuple: Term) -> Result<(), EtsError> {
        let key = self.tuple_key(tuple)?;
        let row = Arc::new(copy_term_to_ets(tuple)?);
        let key = OwnedEtsKey::copy_of(key)?;
        // `replace_entry` swaps in the freshly copied key alongside the row so
        // a retained old key can never outlive its backing row copy.
        match self.entries.entry(key) {
            Entry::Occupied(entry) => {
                let _replaced = entry.replace_entry(row);
            }
            Entry::Vacant(entry) => {
                entry.insert(row);
            }
        }
        Ok(())
    }

    fn lookup(&self, key: Term) -> Vec<Arc<OwnedTerm>> {
        self.entries
            .get(&EtsKey::new(key))
            .map_or_else(Vec::new, |entry| vec![Arc::clone(entry.value())])
    }

    fn delete_key(&self, key: Term) -> bool {
        self.entries.remove(&EtsKey::new(key)).is_some()
    }

    fn delete_object(&self, tuple: Term) -> bool {
        let Ok(key) = self.tuple_key(tuple) else {
            return false;
        };
        self.entries
            .remove_if(&EtsKey::new(key), |_key, value| {
                compare::exact_eq(value.root(), tuple)
            })
            .is_some()
    }

    fn tab2list(&self) -> Vec<Arc<OwnedTerm>> {
        self.entries
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atom::Atom;
    use crate::ets::{EtsTableId, EtsTableType, Protection};
    use crate::term::boxed;
    use std::thread;

    fn metadata(keypos: usize) -> EtsTableMetadata {
        let mut metadata = EtsTableMetadata::new(
            None,
            EtsTableId::from(1_u64),
            EtsTableType::Set,
            Protection::Protected,
            1,
        );
        metadata.keypos = keypos;
        metadata
    }

    fn assert_single_row(rows: &[Arc<OwnedTerm>], expected: Term) {
        assert_eq!(rows.len(), 1);
        assert!(compare::exact_eq(rows[0].root(), expected));
    }

    #[test]
    fn insert_lookup_and_overwrite_by_unique_key() {
        let table = EtsSet::new(metadata(1));
        let mut first_heap = [0_u64; 3];
        let mut second_heap = [0_u64; 3];
        let first =
            boxed::write_tuple(&mut first_heap, &[Term::atom(Atom::OK), Term::small_int(1)])
                .expect("first tuple fits");
        let second = boxed::write_tuple(
            &mut second_heap,
            &[Term::atom(Atom::OK), Term::small_int(2)],
        )
        .expect("second tuple fits");

        table.insert(first).expect("first insert succeeds");
        assert_single_row(&table.lookup(Term::atom(Atom::OK)), first);

        table.insert(second).expect("second insert succeeds");
        assert_single_row(&table.lookup(Term::atom(Atom::OK)), second);
    }

    #[test]
    fn stored_rows_do_not_point_into_the_source_heap() {
        let table = EtsSet::new(metadata(1));
        let mut source_heap = [0_u64; 3];
        let tuple = boxed::write_tuple(
            &mut source_heap,
            &[Term::atom(Atom::OK), Term::small_int(7)],
        )
        .expect("tuple fits");
        table.insert(tuple).expect("insert succeeds");

        // Clobber the source heap: the stored copy must be unaffected.
        source_heap.fill(0);

        let rows = table.lookup(Term::atom(Atom::OK));
        assert_eq!(rows.len(), 1);
        let stored = Tuple::new(rows[0].root()).expect("stored row is a tuple");
        assert_eq!(stored.get(0), Some(Term::atom(Atom::OK)));
        assert_eq!(stored.get(1), Some(Term::small_int(7)));
    }

    #[test]
    fn non_tuple_and_out_of_range_keypos_are_badarg() {
        let table = EtsSet::new(metadata(1));
        assert_eq!(table.insert(Term::small_int(1)), Err(EtsError::Badarg));

        let out_of_range = EtsSet::new(metadata(3));
        let mut heap = [0_u64; 3];
        let tuple = boxed::write_tuple(&mut heap, &[Term::atom(Atom::OK), Term::small_int(1)])
            .expect("tuple fits");
        assert_eq!(out_of_range.insert(tuple), Err(EtsError::Badarg));
    }

    #[test]
    fn delete_key_reports_existence_and_tab2list_returns_all_tuples() {
        let table = EtsSet::new(metadata(1));
        let mut first_heap = [0_u64; 3];
        let mut second_heap = [0_u64; 3];
        let first =
            boxed::write_tuple(&mut first_heap, &[Term::atom(Atom::OK), Term::small_int(1)])
                .expect("first tuple fits");
        let second = boxed::write_tuple(
            &mut second_heap,
            &[Term::atom(Atom::ERROR), Term::small_int(2)],
        )
        .expect("second tuple fits");
        table.insert(first).expect("first insert succeeds");
        table.insert(second).expect("second insert succeeds");

        let listed = table.tab2list();
        assert_eq!(listed.len(), 2);
        for expected in [first, second] {
            assert_eq!(
                listed
                    .iter()
                    .filter(|row| compare::exact_eq(row.root(), expected))
                    .count(),
                1
            );
        }

        assert!(table.delete_key(Term::atom(Atom::OK)));
        assert!(!table.delete_key(Term::atom(Atom::OK)));
        assert!(table.lookup(Term::atom(Atom::OK)).is_empty());
        assert_single_row(&table.lookup(Term::atom(Atom::ERROR)), second);
    }

    #[test]
    fn keypos_is_one_based() {
        let table = EtsSet::new(metadata(2));
        let mut heap = [0_u64; 3];
        let tuple = boxed::write_tuple(&mut heap, &[Term::atom(Atom::OK), Term::small_int(99)])
            .expect("tuple fits");
        table.insert(tuple).expect("insert succeeds");

        assert_single_row(&table.lookup(Term::small_int(99)), tuple);
        assert!(table.lookup(Term::atom(Atom::OK)).is_empty());
    }

    #[test]
    fn write_concurrency_option_allows_concurrent_inserts() {
        let mut metadata = metadata(1);
        metadata.write_concurrency = true;
        let table = Arc::new(EtsSet::new(metadata));
        let handles = (0_i64..16)
            .map(|key| {
                let table = Arc::clone(&table);
                thread::spawn(move || {
                    let mut heap = [0_u64; 3];
                    let tuple = boxed::write_tuple(
                        &mut heap[..],
                        &[Term::small_int(key), Term::small_int(key * 10)],
                    )
                    .expect("tuple fits");
                    table.insert(tuple).expect("insert succeeds");
                    key
                })
            })
            .collect::<Vec<_>>();

        let inserted = handles
            .into_iter()
            .map(|handle| handle.join().expect("writer thread completes"))
            .collect::<Vec<_>>();

        for key in inserted {
            let rows = table.lookup(Term::small_int(key));
            assert_eq!(rows.len(), 1);
            let row = Tuple::new(rows[0].root()).expect("stored row is a tuple");
            assert_eq!(row.get(1), Some(Term::small_int(key * 10)));
        }
    }
}
