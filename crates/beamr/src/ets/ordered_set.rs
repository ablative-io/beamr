//! ETS `ordered_set` table implementation.

use std::{
    collections::BTreeMap,
    ops::Bound::{Excluded, Unbounded},
    sync::{Arc, Mutex, RwLock},
};

use crate::{
    atom::AtomTable,
    ets::{
        EtsError, EtsTable, EtsTableMetadata,
        copy::{OwnedTerm, copy_term_to_ets},
        owned_key::OwnedTermKey,
    },
    term::{Term, boxed::Tuple, compare},
};

use super::TermKey;

type OrderedRows = BTreeMap<OwnedTermKey, Arc<OwnedTerm>>;

/// B-tree backed ETS `ordered_set` table.
///
/// Rows are deep-copied into ETS-owned storage on insert; nothing in the map
/// points into any process heap.
pub struct EtsOrderedSet {
    metadata: EtsTableMetadata,
    atom_table: Arc<AtomTable>,
    rows: OrderedSetRows,
}

enum OrderedSetRows {
    Mutex(Mutex<OrderedRows>),
    RwLock(RwLock<OrderedRows>),
}

impl EtsOrderedSet {
    #[must_use]
    pub fn new(metadata: EtsTableMetadata) -> Self {
        Self::with_atom_table(metadata, Arc::new(AtomTable::with_common_atoms()))
    }

    #[must_use]
    pub fn with_atom_table(metadata: EtsTableMetadata, atom_table: Arc<AtomTable>) -> Self {
        let rows = if metadata.read_concurrency {
            OrderedSetRows::RwLock(RwLock::new(BTreeMap::new()))
        } else {
            OrderedSetRows::Mutex(Mutex::new(BTreeMap::new()))
        };
        Self {
            metadata,
            atom_table,
            rows,
        }
    }

    /// Returns the row holding the smallest key in the table.
    #[must_use]
    pub fn first(&self) -> Option<Arc<OwnedTerm>> {
        self.with_rows(|rows| rows.values().next().map(Arc::clone))
    }

    /// Returns the row holding the largest key in the table.
    #[must_use]
    pub fn last(&self) -> Option<Arc<OwnedTerm>> {
        self.with_rows(|rows| rows.values().next_back().map(Arc::clone))
    }

    /// Returns the row whose key is immediately after `key`, even when `key`
    /// is absent.
    #[must_use]
    pub fn next(&self, key: Term) -> Option<Arc<OwnedTerm>> {
        let key = self.key(key);
        self.with_rows(|rows| {
            rows.range::<TermKey, _>((Excluded(&key), Unbounded))
                .next()
                .map(|(_key, row)| Arc::clone(row))
        })
    }

    /// Returns the row whose key is immediately before `key`, even when `key`
    /// is absent.
    #[must_use]
    pub fn prev(&self, key: Term) -> Option<Arc<OwnedTerm>> {
        let key = self.key(key);
        self.with_rows(|rows| {
            rows.range::<TermKey, _>((Unbounded, Excluded(&key)))
                .next_back()
                .map(|(_key, row)| Arc::clone(row))
        })
    }

    fn key(&self, term: Term) -> TermKey {
        TermKey::with_atom_table(term, Arc::clone(&self.atom_table))
    }

    fn tuple_key(&self, tuple: Term) -> Result<Term, EtsError> {
        let tuple = Tuple::new(tuple).ok_or(EtsError::Badarg)?;
        let index = self
            .metadata
            .keypos
            .checked_sub(1)
            .ok_or(EtsError::Badarg)?;
        tuple.get(index).ok_or(EtsError::Badarg)
    }

    fn with_rows<R>(&self, read: impl FnOnce(&OrderedRows) -> R) -> R {
        match &self.rows {
            OrderedSetRows::Mutex(rows) => match rows.lock() {
                Ok(rows) => read(&rows),
                Err(poisoned) => read(&poisoned.into_inner()),
            },
            OrderedSetRows::RwLock(rows) => match rows.read() {
                Ok(rows) => read(&rows),
                Err(poisoned) => read(&poisoned.into_inner()),
            },
        }
    }

    fn with_rows_mut<R>(&self, write: impl FnOnce(&mut OrderedRows) -> R) -> R {
        match &self.rows {
            OrderedSetRows::Mutex(rows) => match rows.lock() {
                Ok(mut rows) => write(&mut rows),
                Err(poisoned) => write(&mut poisoned.into_inner()),
            },
            OrderedSetRows::RwLock(rows) => match rows.write() {
                Ok(mut rows) => write(&mut rows),
                Err(poisoned) => write(&mut poisoned.into_inner()),
            },
        }
    }
}

impl EtsTable for EtsOrderedSet {
    fn metadata(&self) -> &EtsTableMetadata {
        &self.metadata
    }

    fn insert(&self, tuple: Term) -> Result<(), EtsError> {
        let key = self.tuple_key(tuple)?;
        let row = Arc::new(copy_term_to_ets(tuple)?);
        let key = OwnedTermKey::copy_of(key, Arc::clone(&self.atom_table))?;
        self.with_rows_mut(|rows| {
            // Remove first so the freshly copied key replaces a retained old
            // key: BTreeMap keeps the original key on plain overwrite.
            rows.remove::<OwnedTermKey>(&key);
            rows.insert(key, row);
        });
        Ok(())
    }

    fn lookup(&self, key: Term) -> Vec<Arc<OwnedTerm>> {
        let key = self.key(key);
        self.with_rows(|rows| rows.get(&key).map(Arc::clone).into_iter().collect())
    }

    fn delete_key(&self, key: Term) -> bool {
        let key = self.key(key);
        self.with_rows_mut(|rows| rows.remove(&key).is_some())
    }

    fn delete_object(&self, tuple: Term) -> bool {
        let Ok(key) = self.tuple_key(tuple) else {
            return false;
        };
        let key = self.key(key);
        self.with_rows_mut(|rows| {
            if rows
                .get(&key)
                .is_some_and(|value| compare::exact_eq(value.root(), tuple))
            {
                rows.remove(&key);
                true
            } else {
                false
            }
        })
    }

    fn tab2list(&self) -> Vec<Arc<OwnedTerm>> {
        self.with_rows(|rows| rows.values().map(Arc::clone).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, mpsc},
        thread,
        time::Duration,
    };

    use crate::{
        atom::AtomTable,
        ets::{EtsTable, EtsTableMetadata, EtsTableType, OwnedTerm, Protection},
        term::{Term, boxed::write_tuple, compare},
    };

    use super::EtsOrderedSet;

    fn table(atom_table: Arc<AtomTable>) -> EtsOrderedSet {
        EtsOrderedSet::with_atom_table(
            EtsTableMetadata::new(None, 1, EtsTableType::OrderedSet, Protection::Protected, 42),
            atom_table,
        )
    }

    fn table_with_metadata(
        metadata: EtsTableMetadata,
        atom_table: Arc<AtomTable>,
    ) -> EtsOrderedSet {
        EtsOrderedSet::with_atom_table(metadata, atom_table)
    }

    fn tuple(heap: &mut [u64; 3], key: Term, value: Term) -> Term {
        match write_tuple(&mut heap[..], &[key, value]) {
            Some(term) => term,
            None => unreachable!("test heap has room for a 2-tuple"),
        }
    }

    fn row_key(row: &Arc<OwnedTerm>) -> Term {
        crate::ets::tuple_key(row.root(), 1).expect("stored row has a key")
    }

    fn assert_rows_match(rows: &[Arc<OwnedTerm>], expected: &[Term]) {
        assert_eq!(rows.len(), expected.len());
        for (row, expected) in rows.iter().zip(expected) {
            assert!(compare::exact_eq(row.root(), *expected));
        }
    }

    #[test]
    fn tab2list_returns_tuples_in_key_order() {
        let atom_table = Arc::new(AtomTable::new());
        let table = table(Arc::clone(&atom_table));
        let mut heap_c = Box::new([0; 3]);
        let mut heap_a = Box::new([0; 3]);
        let mut heap_b = Box::new([0; 3]);
        let tuple_c = tuple(
            &mut heap_c,
            Term::small_int(3),
            Term::atom(atom_table.intern("c")),
        );
        let tuple_a = tuple(
            &mut heap_a,
            Term::small_int(1),
            Term::atom(atom_table.intern("a")),
        );
        let tuple_b = tuple(
            &mut heap_b,
            Term::small_int(2),
            Term::atom(atom_table.intern("b")),
        );

        assert_eq!(table.insert(tuple_c), Ok(()));
        assert_eq!(table.insert(tuple_a), Ok(()));
        assert_eq!(table.insert(tuple_b), Ok(()));

        assert_rows_match(&table.tab2list(), &[tuple_a, tuple_b, tuple_c]);
    }

    #[test]
    fn lookup_insert_overwrites_and_delete_key_uses_ordered_key() {
        let atom_table = Arc::new(AtomTable::new());
        let table = table(Arc::clone(&atom_table));
        let mut first_heap = Box::new([0; 3]);
        let mut replacement_heap = Box::new([0; 3]);
        let first = tuple(
            &mut first_heap,
            Term::small_int(1),
            Term::atom(atom_table.intern("first")),
        );
        let replacement = tuple(
            &mut replacement_heap,
            Term::small_int(1),
            Term::atom(atom_table.intern("replacement")),
        );

        assert_eq!(table.insert(first), Ok(()));
        assert_eq!(table.insert(replacement), Ok(()));
        assert_rows_match(&table.lookup(Term::small_int(1)), &[replacement]);
        assert!(table.delete_key(Term::small_int(1)));
        assert!(table.lookup(Term::small_int(1)).is_empty());
        assert!(!table.delete_key(Term::small_int(1)));
    }

    #[test]
    fn insert_rejects_non_tuple_and_missing_keypos() {
        let atom_table = Arc::new(AtomTable::new());
        let mut metadata =
            EtsTableMetadata::new(None, 1, EtsTableType::OrderedSet, Protection::Protected, 42);
        metadata.keypos = 3;
        let table = EtsOrderedSet::with_atom_table(metadata, Arc::clone(&atom_table));
        let mut heap = Box::new([0; 3]);
        let tuple = tuple(
            &mut heap,
            Term::small_int(1),
            Term::atom(atom_table.intern("value")),
        );

        assert_eq!(
            table.insert(Term::small_int(1)),
            Err(crate::ets::EtsError::Badarg)
        );
        assert_eq!(table.insert(tuple), Err(crate::ets::EtsError::Badarg));
    }

    #[test]
    fn ordered_traversal_returns_neighboring_keys_and_boundaries() {
        let atom_table = Arc::new(AtomTable::new());
        let table = table(Arc::clone(&atom_table));
        let mut heap_one = Box::new([0; 3]);
        let mut heap_two = Box::new([0; 3]);
        let mut heap_three = Box::new([0; 3]);
        let one = Term::small_int(1);
        let two = Term::small_int(2);
        let three = Term::small_int(3);

        assert_eq!(
            table.insert(tuple(
                &mut heap_three,
                three,
                Term::atom(atom_table.intern("c")),
            )),
            Ok(())
        );
        assert_eq!(
            table.insert(tuple(
                &mut heap_one,
                one,
                Term::atom(atom_table.intern("a")),
            )),
            Ok(())
        );
        assert_eq!(
            table.insert(tuple(
                &mut heap_two,
                two,
                Term::atom(atom_table.intern("b")),
            )),
            Ok(())
        );

        assert_eq!(table.first().as_ref().map(row_key), Some(one));
        assert_eq!(table.next(one).as_ref().map(row_key), Some(two));
        assert_eq!(
            table.next(Term::small_int(0)).as_ref().map(row_key),
            Some(one)
        );
        assert!(table.next(three).is_none());
        assert_eq!(table.last().as_ref().map(row_key), Some(three));
        assert_eq!(table.prev(three).as_ref().map(row_key), Some(two));
        assert_eq!(
            table.prev(Term::small_int(4)).as_ref().map(row_key),
            Some(three)
        );
        assert!(table.prev(one).is_none());
    }

    #[test]
    fn read_concurrency_uses_shared_read_lock_for_concurrent_lookups() {
        let atom_table = Arc::new(AtomTable::new());
        let mut metadata =
            EtsTableMetadata::new(None, 1, EtsTableType::OrderedSet, Protection::Protected, 42);
        metadata.read_concurrency = true;
        let table = Arc::new(EtsOrderedSet::with_atom_table(
            metadata,
            Arc::clone(&atom_table),
        ));
        let mut heap = Box::new([0; 3]);
        let row = tuple(
            &mut heap,
            Term::small_int(1),
            Term::atom(atom_table.intern("value")),
        );
        assert_eq!(table.insert(row), Ok(()));
        let row_snapshot = crate::ets::copy_term_to_ets(row).expect("snapshot copies");

        let (first_reader_started_tx, first_reader_started_rx) = mpsc::channel();
        let (release_first_reader_tx, release_first_reader_rx) = mpsc::channel();
        let holding_reader = {
            let table = Arc::clone(&table);
            thread::spawn(move || {
                table.with_rows(|rows| {
                    let stored = rows
                        .get(&table.key(Term::small_int(1)))
                        .expect("stored row exists");
                    assert!(compare::exact_eq(stored.root(), row_snapshot.root()));
                    first_reader_started_tx
                        .send(())
                        .expect("test coordinator receives first reader signal");
                    release_first_reader_rx
                        .recv()
                        .expect("test coordinator releases first reader");
                });
            })
        };
        first_reader_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first reader acquired read lock");

        let (concurrent_reader_tx, concurrent_reader_rx) = mpsc::channel();
        let concurrent_reader = {
            let table = Arc::clone(&table);
            thread::spawn(move || {
                concurrent_reader_tx
                    .send(table.lookup(Term::small_int(1)))
                    .expect("test coordinator receives concurrent reader result");
            })
        };
        let concurrent_rows = concurrent_reader_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second reader is not blocked by first reader");
        assert_eq!(concurrent_rows.len(), 1);

        release_first_reader_tx
            .send(())
            .expect("first reader release signal sends");
        holding_reader
            .join()
            .expect("holding reader thread completes");
        concurrent_reader
            .join()
            .expect("concurrent reader thread completes");
    }

    #[test]
    fn read_and_write_concurrency_combination_preserves_ordered_set_semantics() {
        let atom_table = Arc::new(AtomTable::new());
        let mut metadata =
            EtsTableMetadata::new(None, 1, EtsTableType::OrderedSet, Protection::Protected, 42);
        metadata.read_concurrency = true;
        metadata.write_concurrency = true;
        let table = table_with_metadata(metadata, Arc::clone(&atom_table));
        let mut heap_three = Box::new([0; 3]);
        let mut heap_one = Box::new([0; 3]);
        let mut heap_two = Box::new([0; 3]);
        let one = Term::small_int(1);
        let two = Term::small_int(2);
        let three = Term::small_int(3);
        let row_three = tuple(&mut heap_three, three, Term::atom(atom_table.intern("c")));
        let row_one = tuple(&mut heap_one, one, Term::atom(atom_table.intern("a")));
        let row_two = tuple(&mut heap_two, two, Term::atom(atom_table.intern("b")));

        assert_eq!(table.insert(row_three), Ok(()));
        assert_eq!(table.insert(row_one), Ok(()));
        assert_eq!(table.insert(row_two), Ok(()));

        assert_eq!(table.first().as_ref().map(row_key), Some(one));
        assert_eq!(table.next(one).as_ref().map(row_key), Some(two));
        assert_eq!(table.next(two).as_ref().map(row_key), Some(three));
        assert!(table.next(three).is_none());
        assert_eq!(table.last().as_ref().map(row_key), Some(three));
        assert_eq!(table.prev(three).as_ref().map(row_key), Some(two));
        assert_eq!(table.prev(two).as_ref().map(row_key), Some(one));
        assert!(table.prev(one).is_none());
        assert_rows_match(&table.tab2list(), &[row_one, row_two, row_three]);
    }

    #[test]
    fn write_concurrency_without_read_concurrency_keeps_single_ordered_map() {
        let atom_table = Arc::new(AtomTable::new());
        let mut metadata =
            EtsTableMetadata::new(None, 1, EtsTableType::OrderedSet, Protection::Protected, 42);
        metadata.write_concurrency = true;
        let table = table_with_metadata(metadata, Arc::clone(&atom_table));
        let mut heap_one = Box::new([0; 3]);
        let mut heap_two = Box::new([0; 3]);

        assert_eq!(
            table.insert(tuple(
                &mut heap_two,
                Term::small_int(2),
                Term::small_int(20)
            )),
            Ok(())
        );
        assert_eq!(
            table.insert(tuple(
                &mut heap_one,
                Term::small_int(1),
                Term::small_int(10)
            )),
            Ok(())
        );

        assert_eq!(
            table.first().as_ref().map(row_key),
            Some(Term::small_int(1))
        );
        assert_eq!(
            table.next(Term::small_int(1)).as_ref().map(row_key),
            Some(Term::small_int(2))
        );
        assert_eq!(table.last().as_ref().map(row_key), Some(Term::small_int(2)));
    }
}
