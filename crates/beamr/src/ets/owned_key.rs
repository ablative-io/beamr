//! Map keys that own their own ETS copy of the key term.
//!
//! Tables index rows by a key term. If the map key merely pointed into a
//! stored row, replacing or shrinking that row would leave the retained map
//! key dangling (`HashMap`/`BTreeMap` keep the original key on overwrite).
//! These wrappers deep-copy the key into their own [`OwnedTerm`] so a map key
//! is always backed by memory it owns, while hashing, equality, and ordering
//! stay structural — probes built from caller-heap terms compare equal.

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::atom::AtomTable;
use crate::ets::EtsError;
use crate::ets::copy::{OwnedTerm, copy_term_to_ets};
use crate::term::Term;
use crate::term::hash::EtsKey;

use super::TermKey;

/// Hash-map key owning an ETS copy of the key term.
///
/// Probe lookups use [`EtsKey`] directly via `Borrow`, so readers never copy.
pub(crate) struct OwnedEtsKey {
    _owned: OwnedTerm,
    key: EtsKey,
}

impl OwnedEtsKey {
    pub(crate) fn copy_of(key: Term) -> Result<Self, EtsError> {
        let owned = copy_term_to_ets(key)?;
        let key = EtsKey::new(owned.root());
        Ok(Self { _owned: owned, key })
    }
}

impl PartialEq for OwnedEtsKey {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for OwnedEtsKey {}

impl Hash for OwnedEtsKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl Borrow<EtsKey> for OwnedEtsKey {
    fn borrow(&self) -> &EtsKey {
        &self.key
    }
}

/// Ordered-map key owning an ETS copy of the key term.
///
/// Probe lookups and range scans use [`TermKey`] directly via `Borrow`.
pub(crate) struct OwnedTermKey {
    _owned: OwnedTerm,
    key: TermKey,
}

impl OwnedTermKey {
    pub(crate) fn copy_of(key: Term, atom_table: Arc<AtomTable>) -> Result<Self, EtsError> {
        let owned = copy_term_to_ets(key)?;
        let key = TermKey::with_atom_table(owned.root(), atom_table);
        Ok(Self { _owned: owned, key })
    }
}

impl PartialEq for OwnedTermKey {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for OwnedTermKey {}

impl PartialOrd for OwnedTermKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OwnedTermKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

impl Borrow<TermKey> for OwnedTermKey {
    fn borrow(&self) -> &TermKey {
        &self.key
    }
}
