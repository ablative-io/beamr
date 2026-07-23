use std::sync::Arc;

use dashmap::{DashMap, mapref::entry::Entry};

use crate::ets::copy::{OwnedTerm, copy_term_to_ets};
use crate::ets::owned_key::OwnedEtsKey;
use crate::term::{Term, compare, hash::EtsKey};

use super::{EtsError, EtsTable, EtsTableMetadata, tuple_key};

type BagStorage = DashMap<OwnedEtsKey, Vec<Arc<OwnedTerm>>>;

/// ETS bag table storage: many distinct tuples per key.
///
/// Rows are deep-copied into ETS-owned storage on insert; nothing in the map
/// points into any process heap.
pub struct EtsBag {
    metadata: EtsTableMetadata,
    storage: BagStorage,
}

impl EtsBag {
    #[must_use]
    pub fn new(metadata: EtsTableMetadata) -> Self {
        Self {
            metadata,
            storage: DashMap::new(),
        }
    }
}

impl EtsTable for EtsBag {
    fn metadata(&self) -> &EtsTableMetadata {
        &self.metadata
    }

    fn insert(&self, tuple: Term) -> Result<(), EtsError> {
        insert_bag_tuple(
            &self.storage,
            tuple_key(tuple, self.metadata.keypos)?,
            tuple,
            false,
        )
    }

    fn lookup(&self, key: Term) -> Vec<Arc<OwnedTerm>> {
        lookup_key(&self.storage, key)
    }

    fn delete_key(&self, key: Term) -> bool {
        delete_key(&self.storage, key)
    }

    fn delete_object(&self, tuple: Term) -> bool {
        delete_object(&self.storage, tuple, self.metadata.keypos)
    }

    fn tab2list(&self) -> Vec<Arc<OwnedTerm>> {
        tab2list(&self.storage)
    }
}

/// ETS duplicate_bag table storage: many tuples per key, preserving duplicates.
pub struct EtsDuplicateBag {
    metadata: EtsTableMetadata,
    storage: BagStorage,
}

impl EtsDuplicateBag {
    #[must_use]
    pub fn new(metadata: EtsTableMetadata) -> Self {
        Self {
            metadata,
            storage: DashMap::new(),
        }
    }
}

impl EtsTable for EtsDuplicateBag {
    fn metadata(&self) -> &EtsTableMetadata {
        &self.metadata
    }

    fn insert(&self, tuple: Term) -> Result<(), EtsError> {
        insert_bag_tuple(
            &self.storage,
            tuple_key(tuple, self.metadata.keypos)?,
            tuple,
            true,
        )
    }

    fn lookup(&self, key: Term) -> Vec<Arc<OwnedTerm>> {
        lookup_key(&self.storage, key)
    }

    fn delete_key(&self, key: Term) -> bool {
        delete_key(&self.storage, key)
    }

    fn delete_object(&self, tuple: Term) -> bool {
        delete_object(&self.storage, tuple, self.metadata.keypos)
    }

    fn tab2list(&self) -> Vec<Arc<OwnedTerm>> {
        tab2list(&self.storage)
    }
}

fn insert_bag_tuple(
    storage: &BagStorage,
    key: Term,
    tuple: Term,
    allow_duplicates: bool,
) -> Result<(), EtsError> {
    let row = Arc::new(copy_term_to_ets(tuple)?);
    let key = OwnedEtsKey::copy_of(key)?;
    match storage.entry(key) {
        Entry::Occupied(mut entry) => {
            let values = entry.get_mut();
            if allow_duplicates
                || !values
                    .iter()
                    .any(|value| compare::exact_eq(value.root(), row.root()))
            {
                values.push(row);
            }
        }
        Entry::Vacant(entry) => {
            entry.insert(vec![row]);
        }
    }
    Ok(())
}

fn lookup_key(storage: &BagStorage, key: Term) -> Vec<Arc<OwnedTerm>> {
    storage
        .get(&EtsKey::new(key))
        .map_or_else(Vec::new, |entry| entry.value().clone())
}

fn delete_key(storage: &BagStorage, key: Term) -> bool {
    storage.remove(&EtsKey::new(key)).is_some()
}

fn delete_object(storage: &BagStorage, tuple: Term, keypos: usize) -> bool {
    let Ok(key) = tuple_key(tuple, keypos) else {
        return false;
    };
    let ets_key = EtsKey::new(key);
    let (deleted, remove_bucket) = match storage.get_mut(&ets_key) {
        Some(mut entry) => {
            let values = entry.value_mut();
            let original_len = values.len();
            values.retain(|value| !compare::exact_eq(value.root(), tuple));
            (values.len() != original_len, values.is_empty())
        }
        None => (false, false),
    };
    if remove_bucket {
        storage.remove(&ets_key);
    }
    deleted
}

fn tab2list(storage: &BagStorage) -> Vec<Arc<OwnedTerm>> {
    storage
        .iter()
        .flat_map(|entry| entry.value().clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{EtsBag, EtsDuplicateBag};
    use crate::{
        atom::Atom,
        ets::{EtsError, EtsTable, EtsTableMetadata, EtsTableType, OwnedTerm, Protection},
        term::{Term, boxed::write_tuple, compare},
    };
    use std::sync::Arc;

    fn metadata(table_type: EtsTableType) -> EtsTableMetadata {
        EtsTableMetadata::new(None, 1, table_type, Protection::Public, 7)
    }

    fn tuple_with_terms(words: &mut [u64], key: Term, value: Term) -> Term {
        let elements = [key, value];
        match write_tuple(words, &elements) {
            Some(term) => term,
            None => panic!("test tuple backing storage is too small"),
        }
    }

    fn tuple(words: &mut [u64], key: Atom, value: i64) -> Term {
        tuple_with_terms(words, Term::atom(key), Term::small_int(value))
    }

    fn count_matching(rows: &[Arc<OwnedTerm>], expected: Term) -> usize {
        rows.iter()
            .filter(|row| compare::exact_eq(row.root(), expected))
            .count()
    }

    #[test]
    fn bag_lookup_returns_all_tuples_for_key() {
        let table = EtsBag::new(metadata(EtsTableType::Bag));
        let mut first_words = [0_u64; 3];
        let mut second_words = [0_u64; 3];
        let first = tuple(&mut first_words, Atom::OK, 1);
        let second = tuple(&mut second_words, Atom::OK, 2);

        assert_eq!(table.insert(first), Ok(()));
        assert_eq!(table.insert(second), Ok(()));

        let values = table.lookup(Term::atom(Atom::OK));
        assert_eq!(values.len(), 2);
        assert_eq!(count_matching(&values, first), 1);
        assert_eq!(count_matching(&values, second), 1);
    }

    #[test]
    fn bag_rejects_exact_duplicate_tuple() {
        let table = EtsBag::new(metadata(EtsTableType::Bag));
        let mut tuple_words = [0_u64; 3];
        let item = tuple(&mut tuple_words, Atom::OK, 1);

        assert_eq!(table.insert(item), Ok(()));
        assert_eq!(table.insert(item), Ok(()));

        let values = table.lookup(Term::atom(Atom::OK));
        assert_eq!(values.len(), 1);
        assert_eq!(count_matching(&values, item), 1);
    }

    #[test]
    fn duplicate_bag_preserves_exact_duplicates() {
        let table = EtsDuplicateBag::new(metadata(EtsTableType::DuplicateBag));
        let mut tuple_words = [0_u64; 3];
        let item = tuple(&mut tuple_words, Atom::OK, 1);

        assert_eq!(table.insert(item), Ok(()));
        assert_eq!(table.insert(item), Ok(()));

        let values = table.lookup(Term::atom(Atom::OK));
        assert_eq!(values.len(), 2);
        assert_eq!(count_matching(&values, item), 2);
    }

    #[test]
    fn delete_key_removes_all_tuples_for_key() {
        let table = EtsDuplicateBag::new(metadata(EtsTableType::DuplicateBag));
        let mut first_words = [0_u64; 3];
        let mut second_words = [0_u64; 3];
        let first = tuple(&mut first_words, Atom::OK, 1);
        let second = tuple(&mut second_words, Atom::OK, 2);

        assert_eq!(table.insert(first), Ok(()));
        assert_eq!(table.insert(second), Ok(()));

        assert!(table.delete_key(Term::atom(Atom::OK)));
        assert!(table.lookup(Term::atom(Atom::OK)).is_empty());
        assert!(!table.delete_key(Term::atom(Atom::OK)));
    }

    #[test]
    fn delete_key_only_removes_requested_key() {
        let table = EtsBag::new(metadata(EtsTableType::Bag));
        let mut first_words = [0_u64; 3];
        let mut second_words = [0_u64; 3];
        let first = tuple(&mut first_words, Atom::OK, 1);
        let second = tuple(&mut second_words, Atom::ERROR, 2);

        assert_eq!(table.insert(first), Ok(()));
        assert_eq!(table.insert(second), Ok(()));

        assert!(table.delete_key(Term::atom(Atom::OK)));

        assert!(table.lookup(Term::atom(Atom::OK)).is_empty());
        let remaining = table.lookup(Term::atom(Atom::ERROR));
        assert_eq!(remaining.len(), 1);
        assert_eq!(count_matching(&remaining, second), 1);
    }

    #[test]
    fn non_tuple_insert_returns_badarg() {
        let table = EtsBag::new(metadata(EtsTableType::Bag));

        assert_eq!(table.insert(Term::small_int(1)), Err(EtsError::Badarg));
    }

    #[test]
    fn out_of_range_key_position_returns_badarg() {
        let table = EtsBag::new(EtsTableMetadata {
            keypos: 3,
            ..metadata(EtsTableType::Bag)
        });
        let mut tuple_words = [0_u64; 3];
        let item = tuple(&mut tuple_words, Atom::OK, 1);

        assert_eq!(table.insert(item), Err(EtsError::Badarg));
    }

    #[test]
    fn zero_key_position_returns_badarg() {
        let table = EtsDuplicateBag::new(EtsTableMetadata {
            keypos: 0,
            ..metadata(EtsTableType::DuplicateBag)
        });
        let mut tuple_words = [0_u64; 3];
        let item = tuple(&mut tuple_words, Atom::OK, 1);

        assert_eq!(table.insert(item), Err(EtsError::Badarg));
    }

    #[test]
    fn bag_uses_configured_key_position() {
        let table = EtsBag::new(EtsTableMetadata {
            keypos: 2,
            ..metadata(EtsTableType::Bag)
        });
        let mut tuple_words = [0_u64; 3];
        let item = tuple_with_terms(&mut tuple_words, Term::atom(Atom::OK), Term::small_int(42));

        assert_eq!(table.insert(item), Ok(()));

        let values = table.lookup(Term::small_int(42));
        assert_eq!(values.len(), 1);
        assert_eq!(count_matching(&values, item), 1);
        assert!(table.lookup(Term::atom(Atom::OK)).is_empty());
    }
}
