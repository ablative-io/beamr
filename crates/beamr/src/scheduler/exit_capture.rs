//! Owned capture of process exit values.
//!
//! A process heap is torn down shortly after the process exits, but exit
//! results and exceptions are read later — by `run_until_exit` callers, the
//! CLI formatter, and supervision diagnostics. Storing raw heap terms at the
//! exit boundary therefore produces dangling pointers. This module deep-copies
//! exit values into self-owned allocations while the source heap is still
//! alive, so consumers can hold them indefinitely.

use crate::atom::Atom;
use crate::ets::copy::{OwnedTerm, copy_term_to_ets};
use crate::process::Exception;
use crate::term::Term;

/// Deep-copies `term` into self-owned storage.
///
/// Falls back to the `undefined` atom for terms that cannot be represented
/// outside their source heap (for example an exotic boxed kind the copier
/// does not support); exit reporting must never tear down the scheduler.
pub(super) fn capture_term(term: Term) -> OwnedTerm {
    copy_term_to_ets(term).unwrap_or_else(|_| OwnedTerm::immediate(Term::atom(Atom::UNDEFINED)))
}

/// An exception whose class, reason, and stacktrace are owned by the capture
/// rather than borrowed from a process heap.
#[derive(Debug)]
pub struct OwnedException {
    class: OwnedTerm,
    reason: OwnedTerm,
    stacktrace: OwnedTerm,
}

impl OwnedException {
    /// Captures `exception` by deep-copying each field out of the process heap.
    pub(super) fn capture(exception: Exception) -> Self {
        Self {
            class: capture_term(exception.class),
            reason: capture_term(exception.reason),
            stacktrace: capture_term(exception.stacktrace),
        }
    }

    /// Borrowed [`Exception`] view over the owned terms.
    ///
    /// The returned terms are valid for as long as this capture is alive.
    #[must_use]
    pub fn view(&self) -> Exception {
        Exception {
            class: self.class.root(),
            reason: self.reason.root(),
            stacktrace: self.stacktrace.root(),
        }
    }

    /// Formats exception details for user-facing diagnostics.
    #[must_use]
    pub fn format_with_atoms(&self, atom_table: &crate::atom::AtomTable) -> String {
        self.view().format_with_atoms(atom_table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Process;

    #[test]
    fn captured_list_survives_source_heap_teardown() {
        let mut process = Process::new(1, 64);
        let heap = process.heap_mut();
        let block = heap.alloc_slice(2).expect("heap alloc");
        let cons = crate::term::boxed::write_cons(block, Term::small_int(7), Term::NIL)
            .expect("cons write");

        let captured = capture_term(cons);
        drop(process);

        let copied = crate::term::boxed::Cons::new(captured.root()).expect("owned cons");
        assert_eq!(copied.head().as_small_int(), Some(7));
        assert!(copied.tail().is_nil());
    }

    #[test]
    fn captured_exception_views_owned_terms() {
        let mut process = Process::new(1, 64);
        let heap = process.heap_mut();
        let block = heap.alloc_slice(2).expect("heap alloc");
        let reason = crate::term::boxed::write_cons(block, Term::small_int(1), Term::NIL)
            .expect("cons write");
        let exception = Exception {
            class: Term::atom(Atom::ERROR),
            reason,
            stacktrace: Term::NIL,
        };

        let captured = OwnedException::capture(exception);
        drop(process);

        let view = captured.view();
        assert_eq!(view.class, Term::atom(Atom::ERROR));
        let copied = crate::term::boxed::Cons::new(view.reason).expect("owned reason");
        assert_eq!(copied.head().as_small_int(), Some(1));
    }
}
