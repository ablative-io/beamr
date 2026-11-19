//! The process dictionary, keyed by Erlang exact equality.

use crate::atom::Atom;
use crate::term::{Term, compare};

use super::Process;

impl Process {
    /// Store `value` under `key` in the process dictionary.
    ///
    /// Existing keys are matched with Erlang exact equality (`=:=`). Returns the
    /// previous value, or `undefined` when the key was not present.
    pub fn dict_put(&mut self, key: Term, value: Term) -> Term {
        for (existing_key, existing_value) in &mut self.dictionary {
            if compare::exact_eq(*existing_key, key) {
                let old_value = *existing_value;
                *existing_value = value;
                return old_value;
            }
        }

        self.dictionary.push((key, value));
        Term::atom(Atom::UNDEFINED)
    }

    /// Fetch a value from the process dictionary by exact-equality key match.
    #[must_use]
    pub fn dict_get(&self, key: Term) -> Term {
        self.dictionary
            .iter()
            .find_map(|(existing_key, value)| {
                compare::exact_eq(*existing_key, key).then_some(*value)
            })
            .unwrap_or_else(|| Term::atom(Atom::UNDEFINED))
    }

    /// Borrow all process dictionary entries in current vector order.
    #[must_use]
    pub fn dict_get_all(&self) -> &[(Term, Term)] {
        &self.dictionary
    }

    /// Remove a dictionary entry by exact-equality key match.
    ///
    /// Uses `swap_remove`, so entry order may change after deletion.
    pub fn dict_erase(&mut self, key: Term) -> Term {
        let Some(index) = self
            .dictionary
            .iter()
            .position(|(existing_key, _)| compare::exact_eq(*existing_key, key))
        else {
            return Term::atom(Atom::UNDEFINED);
        };

        let (_key, value) = self.dictionary.swap_remove(index);
        value
    }

    /// Remove and return all process dictionary entries.
    pub fn dict_erase_all(&mut self) -> Vec<(Term, Term)> {
        std::mem::take(&mut self.dictionary)
    }

    /// Return all keys whose values exactly match `value`.
    #[must_use]
    pub fn dict_get_keys(&self, value: Term) -> Vec<Term> {
        self.dictionary
            .iter()
            .filter_map(|(key, existing_value)| {
                compare::exact_eq(*existing_value, value).then_some(*key)
            })
            .collect()
    }
}
