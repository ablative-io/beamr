use std::hash::{Hash, Hasher};

use dashmap::DashMap;

use crate::term::Term;

use super::{EtsError, EtsTable, EtsTableMetadata, tuple_key};

/// Hashable ETS key wrapper using Erlang exact equality semantics.
#[derive(Copy, Clone, Debug)]
pub struct EtsKey(pub Term);

impl PartialEq for EtsKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for EtsKey {}

impl Hash for EtsKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // `Term` equality uses Erlang exact equality and can consider separately
        // allocated boxed terms equal. Until ETS owns a full structural hash
        // alongside deep-copy storage, a single bucket preserves the Hash/Eq
        // contract for all term shapes rather than hashing pointer addresses.
        let _ = self.0;
        0_u8.hash(state);
    }
}

/// ETS set table storage: one tuple per key, replacing the previous tuple.
pub struct EtsSet {
    metadata: EtsTableMetadata,
    storage: DashMap<EtsKey, Term>,
}

impl EtsSet {
    #[must_use]
    pub fn new(metadata: EtsTableMetadata) -> Self {
        Self {
            metadata,
            storage: DashMap::new(),
        }
    }
}

impl EtsTable for EtsSet {
    fn metadata(&self) -> &EtsTableMetadata {
        &self.metadata
    }

    fn insert(&self, tuple: Term) -> Result<(), EtsError> {
        let key = tuple_key(tuple, self.metadata.keypos)?;
        self.storage.insert(EtsKey(key), tuple);
        Ok(())
    }

    fn lookup(&self, key: Term) -> Vec<Term> {
        self.storage
            .get(&EtsKey(key))
            .map_or_else(Vec::new, |entry| vec![*entry.value()])
    }

    fn delete_key(&self, key: Term) -> bool {
        self.storage.remove(&EtsKey(key)).is_some()
    }

    fn tab2list(&self) -> Vec<Term> {
        self.storage.iter().map(|entry| *entry.value()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::EtsSet;
    use crate::{
        atom::Atom,
        ets::{EtsTable, EtsTableMetadata, EtsTableType, Protection},
        term::{Term, boxed::write_tuple},
    };

    fn metadata() -> EtsTableMetadata {
        EtsTableMetadata {
            name: None,
            id: 1,
            table_type: EtsTableType::Set,
            protection: Protection::Public,
            owner: 7,
            keypos: 1,
        }
    }

    fn tuple(words: &mut [u64], key: Atom, value: i64) -> Term {
        let elements = [Term::atom(key), Term::small_int(value)];
        match write_tuple(words, &elements) {
            Some(term) => term,
            None => panic!("test tuple backing storage is too small"),
        }
    }

    #[test]
    fn set_replaces_existing_tuple_for_key() {
        let table = EtsSet::new(metadata());
        let mut first_words = [0_u64; 3];
        let mut second_words = [0_u64; 3];
        let first = tuple(&mut first_words, Atom::OK, 1);
        let second = tuple(&mut second_words, Atom::OK, 2);

        assert_eq!(table.insert(first), Ok(()));
        assert_eq!(table.insert(second), Ok(()));

        assert_eq!(table.lookup(Term::atom(Atom::OK)), vec![second]);
    }
}
