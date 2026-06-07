//! Erlang Term Storage table abstractions.

pub mod bag;
pub mod set;

use std::{error::Error, fmt, sync::Arc};

use crate::{atom::Atom, term::Term};

pub use bag::{EtsBag, EtsDuplicateBag};
pub use set::{EtsKey, EtsSet};

/// Opaque ETS table identifier allocated by the table registry.
pub type EtsTableId = u64;

/// ETS table kind.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EtsTableType {
    Set,
    OrderedSet,
    Bag,
    DuplicateBag,
}

/// Access protection configured for an ETS table.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Protection {
    Public,
    Protected,
    Private,
}

/// Metadata shared by all ETS table implementations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EtsTableMetadata {
    pub name: Option<Atom>,
    pub id: EtsTableId,
    pub table_type: EtsTableType,
    pub protection: Protection,
    pub owner: u64,
    /// One-based tuple element position used as the table key.
    pub keypos: usize,
}

/// Errors produced by ETS operations.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EtsError {
    Badarg,
    UnsupportedTableType(EtsTableType),
}

impl fmt::Display for EtsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Badarg => formatter.write_str("bad ETS argument"),
            Self::UnsupportedTableType(table_type) => {
                write!(formatter, "unsupported ETS table type {table_type:?}")
            }
        }
    }
}

impl Error for EtsError {}

/// Common interface implemented by concrete ETS table storage types.
pub trait EtsTable: Send + Sync {
    fn metadata(&self) -> &EtsTableMetadata;
    fn insert(&self, tuple: Term) -> Result<(), EtsError>;
    fn lookup(&self, key: Term) -> Vec<Term>;
    fn delete_key(&self, key: Term) -> bool;
    fn tab2list(&self) -> Vec<Term>;
}

/// Create concrete storage for ETS table metadata.
pub fn create_table(metadata: EtsTableMetadata) -> Result<Arc<dyn EtsTable>, EtsError> {
    match metadata.table_type {
        EtsTableType::Set => Ok(Arc::new(EtsSet::new(metadata))),
        EtsTableType::Bag => Ok(Arc::new(EtsBag::new(metadata))),
        EtsTableType::DuplicateBag => Ok(Arc::new(EtsDuplicateBag::new(metadata))),
        EtsTableType::OrderedSet => Err(EtsError::UnsupportedTableType(EtsTableType::OrderedSet)),
    }
}

pub(crate) fn tuple_key(tuple: Term, keypos: usize) -> Result<Term, EtsError> {
    let index = keypos.checked_sub(1).ok_or(EtsError::Badarg)?;
    let tuple = crate::term::boxed::Tuple::new(tuple).ok_or(EtsError::Badarg)?;
    tuple.get(index).ok_or(EtsError::Badarg)
}

#[cfg(test)]
mod tests {
    use super::{EtsError, EtsTableMetadata, EtsTableType, Protection, create_table};
    use crate::{
        atom::Atom,
        term::{Term, boxed::write_tuple},
    };

    fn metadata(table_type: EtsTableType) -> EtsTableMetadata {
        EtsTableMetadata {
            name: None,
            id: 1,
            table_type,
            protection: Protection::Public,
            owner: 7,
            keypos: 1,
        }
    }

    fn create_test_table(table_type: EtsTableType) -> std::sync::Arc<dyn super::EtsTable> {
        match create_table(metadata(table_type)) {
            Ok(table) => table,
            Err(error) => panic!("{table_type:?} table creation failed: {error}"),
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
    fn create_table_instantiates_bag_with_duplicate_rejection() {
        let table = create_test_table(EtsTableType::Bag);
        let mut tuple_words = [0_u64; 3];
        let item = tuple(&mut tuple_words, Atom::OK, 1);

        assert_eq!(table.insert(item), Ok(()));
        assert_eq!(table.insert(item), Ok(()));

        assert_eq!(table.lookup(Term::atom(Atom::OK)), vec![item]);
    }

    #[test]
    fn create_table_instantiates_bag_with_multi_value_lookup() {
        let table = create_test_table(EtsTableType::Bag);
        let mut first_words = [0_u64; 3];
        let mut second_words = [0_u64; 3];
        let first = tuple(&mut first_words, Atom::OK, 1);
        let second = tuple(&mut second_words, Atom::OK, 2);

        assert_eq!(table.insert(first), Ok(()));
        assert_eq!(table.insert(second), Ok(()));

        let values = table.lookup(Term::atom(Atom::OK));
        assert_eq!(values.len(), 2);
        assert!(values.contains(&first));
        assert!(values.contains(&second));
    }

    #[test]
    fn create_table_instantiates_duplicate_bag_with_multiplicity() {
        let table = create_test_table(EtsTableType::DuplicateBag);
        let mut tuple_words = [0_u64; 3];
        let item = tuple(&mut tuple_words, Atom::OK, 1);

        assert_eq!(table.insert(item), Ok(()));
        assert_eq!(table.insert(item), Ok(()));

        assert_eq!(table.lookup(Term::atom(Atom::OK)), vec![item, item]);
    }

    #[test]
    fn create_table_rejects_ordered_set_until_implemented() {
        assert_eq!(
            create_table(metadata(EtsTableType::OrderedSet)).err(),
            Some(EtsError::UnsupportedTableType(EtsTableType::OrderedSet))
        );
    }
}
