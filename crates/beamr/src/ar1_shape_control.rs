//! AR-1 R10 — the control fixtures for `shape_hunt.py`, one per shape class.
//!
//! This module ships no behaviour. It exists so the accumulator-rooting hunt
//! at `docs/design/beamr/briefs/evidence/accumulator-rooting/shape_hunt.py`
//! has a **known positive it can still find after the lane it grades has
//! succeeded**.
//!
//! # Why the control could not stay on a live defect
//!
//! The hunt's original control keyed on `code_management_bifs.rs`'s
//! `list = context.alloc_cons(tuple, list)?` — RF-006 defect #1. A control
//! keyed on a live defect is destroyed by the patch. Re-siting it on "a
//! synthetic instance of the shape" was worse: the gate's own remedy criterion
//! is *the shape CANNOT BE WRITTEN*, so a structural remedy stops the fixture
//! compiling and **the control dies by the remedy SUCCEEDING** — it fires on
//! success, which is the failure mode that reads as a finding.
//!
//! # Why these fixtures survive both remedies
//!
//! `shape_hunt.py` is **purely syntactic**: `ALLOC` is a regex over method-name
//! spelling. It resolves no types and consults no API. So the control's target
//! must depend only on what the instrument reads — bind syntax and spelling —
//! and on **none of the semantics a remedy changes**:
//!
//! * **Type-level remedy** (rooted handles, a `Rooted<Term>` wrapper, an API
//!   that cannot be misused): defeated, because [`FixtureHeap`] and
//!   [`FixtureTerm`] are local types with no relationship to `crate::term::Term`
//!   or the real allocator. A change to the real types cannot reach them.
//! * **Lint-on-spelling remedy**: defeated by the suppressions carried here —
//!   `clippy::disallowed_methods` at the module, and a bare `ast-grep-ignore`
//!   on each matching line. The bare form is deliberate: a future rule's id is
//!   not knowable today, and a named suppression for a rule that does not exist
//!   yet is a guard written for one arm and invisible to the other.
//!
//! # Declared coupling — this rots silently if it is not read
//!
//! ⚠️ These fixtures work **because the hunt is a regex and is blind to `cfg`
//! and to attributes**. That blindness is deliberately load-bearing: a positive
//! sited inside `#[cfg(test)]` is visible to the instrument yet absent from the
//! shipped binary. **If `shape_hunt.py` ever becomes AST-based or `cfg`-aware,
//! this fixture stops being visible to it and the control must be re-sited.**
//!
//! ⚠️ The file must also stay inside the walked population: `source_files()`
//! walks `crates/**/*.rs` and skips `tests.rs`, `*_tests.rs` and
//! `src/native/context/`. **The instinctive home for a fixture — a test file —
//! is the one place the instrument is blind.** Renaming this file to anything
//! ending `_tests.rs` silently removes all five controls.
//!
//! ⚠️ `cargo fmt` is the third hazard, and it is the only one that is checked
//! rather than merely stated: the detector regexes are per-line, so a reflow
//! that splits a matching line destroys that class's control. The battery runs
//! `cargo fmt --all` before the hunt, so a fixture broken by a reflow turns its
//! own per-class control RED on the next run.
//!
//! # Accounting
//!
//! All five are registered `CONTROL-FIXTURE` in the lane's disposition ledger,
//! `docs/design/beamr/briefs/evidence/accumulator-rooting/dispositions.json`,
//! and excluded from remediation **by exact literal path** — denominator one,
//! auditable, incapable of swallowing a real site. They are **labelled, never
//! filtered**: the hunt reports them in its `cfg(test)` column, because a hunt
//! that hides its own control certifies nothing.

// The `#[cfg(test)]` below is what the hunt's labeller keys on — a literal
// attribute followed by a brace. It is what puts these lines in the
// `cfg(test)` column instead of the production count, so it is load-bearing
// for the instrument, not just for the compiler.
#[cfg(test)]
mod fixtures {
    // A remedy that bans the `alloc_*` spellings via clippy must not take the
    // control with it. See the module docs: the arm this covers is the
    // lint-shaped remedy, and it is covered here rather than at each call so
    // that a future rule of any name is caught by one suppression.
    #![allow(clippy::disallowed_methods)]

    #[cfg(not(feature = "std"))]
    use alloc::vec::Vec;

    /// A term-shaped value that is deliberately **not** `crate::term::Term`.
    ///
    /// The hunt resolves no types, so this is indistinguishable from the real
    /// thing to the instrument — and untouchable by a type-level remedy, which
    /// is the entire point of the fixture.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct FixtureTerm(usize);

    /// The always-empty tail used by the list-shaped fixtures.
    const NIL: FixtureTerm = FixtureTerm(0);

    /// A local allocator carrying the **method spellings** the hunt greps for
    /// and none of the real allocator's semantics.
    #[derive(Default)]
    struct FixtureHeap {
        cells: usize,
    }

    impl FixtureHeap {
        /// Spelling-compatible with the real `alloc_cons`; semantically inert.
        fn alloc_cons(&mut self, head: FixtureTerm, tail: FixtureTerm) -> FixtureTerm {
            self.cells += 1;
            FixtureTerm(head.0 ^ tail.0 ^ self.cells)
        }

        /// Spelling-compatible with the real `alloc_tuple`; semantically inert.
        fn alloc_tuple(&mut self, parts: &[FixtureTerm]) -> FixtureTerm {
            self.cells += parts.len();
            FixtureTerm(parts.iter().fold(self.cells, |acc, p| acc ^ p.0))
        }

        /// Spelling-compatible with the real `alloc_list`; semantically inert.
        fn alloc_list(&mut self, items: &[FixtureTerm]) -> FixtureTerm {
            self.cells += items.len();
            FixtureTerm(items.iter().fold(self.cells, |acc, p| acc ^ p.0))
        }

        /// How many cells this heap has handed out, so the `#[test]` below can
        /// assert the fixtures actually ran rather than merely compiled.
        fn cells(&self) -> usize {
            self.cells
        }
    }

    /// **S3a** — `.map(..)` into a collection. The hunt's binder keys on
    /// `Vec::new` plus `.push`, so a `.collect()` carrier is invisible to it.
    fn s3a_collect_carrier(heap: &mut FixtureHeap, src: &[FixtureTerm]) -> FixtureTerm {
        // ast-grep-ignore
        let s3a_mapped: Vec<_> = src.iter().map(|s| heap.alloc_cons(*s, NIL)).collect();
        heap.alloc_list(&s3a_mapped)
    }

    /// **S3b** — bind from a `match`/`if` expression. The right-hand side is an
    /// expression rather than a call, so a callee-shaped binder never sees it.
    ///
    /// It matches on a count rather than a `bool` deliberately: `match` on a
    /// two-valued type is `clippy::match_bool`, and a fixture that only exists
    /// while it compiles must not carry an avoidable lint into a lane whose
    /// likely remedy is itself a lint.
    fn s3b_match_expression_carrier(heap: &mut FixtureHeap, arms: usize) -> FixtureTerm {
        let held = FixtureTerm(7);
        let s3b_chosen = match arms {
            0 => held,
            // ast-grep-ignore
            _ => heap.alloc_cons(held, NIL),
        };
        heap.alloc_tuple(&[s3b_chosen, held])
    }

    /// **S3c** — bind from an array/tuple literal. The carrier is an element
    /// inside the literal, never named on its own.
    fn s3c_literal_carrier(heap: &mut FixtureHeap, head: FixtureTerm) -> FixtureTerm {
        // ast-grep-ignore
        let s3c_pair = [heap.alloc_cons(head, NIL), head];
        heap.alloc_tuple(&s3c_pair)
    }

    /// **S3d** — reassignment with no `let`. This is RF-006 defect #1's exact
    /// shape, and the class the hunt's original live-defect control stood on.
    fn s3d_reassignment_carrier(heap: &mut FixtureHeap, items: &[FixtureTerm]) -> FixtureTerm {
        let mut s3d_acc = NIL;
        for item in items {
            // ast-grep-ignore
            s3d_acc = heap.alloc_cons(*item, s3d_acc);
        }
        s3d_acc
    }

    /// **S3e** — a `Vec` OF TUPLES. The carrier is an element inside a tuple
    /// inside a `Vec`, so it is neither a `Vec<Term>` nor a term-shaped push
    /// argument. This is the class that took the lane from 14 crossings to 17.
    fn s3e_vec_of_tuples_carrier(heap: &mut FixtureHeap, items: &[FixtureTerm]) -> FixtureTerm {
        let mut s3e_pairs: Vec<(FixtureTerm, FixtureTerm)> = Vec::with_capacity(items.len());
        for item in items {
            // ast-grep-ignore
            s3e_pairs.push((heap.alloc_cons(*item, NIL), *item));
        }
        // NOTE: this line is an INCIDENTAL S3a match -- a `.map(..).collect()`
        // inside the S3e fixture. It is left in place deliberately: it is why
        // the per-class controls key on the `s3aN_`-style binding name rather
        // than on "any hit of this class in the fixture file". Keyed the loose
        // way, THIS LINE WOULD HAVE KEPT S3a GREEN AFTER THE S3a FIXTURE BROKE.
        let flat: Vec<FixtureTerm> = s3e_pairs.iter().map(|p| p.0).collect();
        heap.alloc_list(&flat)
    }

    /// Exercises all five fixtures.
    ///
    /// Its job is not to assert behaviour — the fixtures have none worth
    /// asserting. It is to keep them **compiled and called**, so that a fixture
    /// broken by a refactor fails the build instead of quietly ceasing to be a
    /// control. A fixture that stops compiling is loud; one that stops being
    /// referenced is silent, and silence is what this whole row exists to
    /// prevent.
    #[test]
    fn every_shape_class_fixture_is_live() {
        let mut heap = FixtureHeap::default();
        let seed = [FixtureTerm(1), FixtureTerm(2), FixtureTerm(3)];

        let a = s3a_collect_carrier(&mut heap, &seed);
        let b = s3b_match_expression_carrier(&mut heap, 1);
        let c = s3c_literal_carrier(&mut heap, seed[0]);
        let d = s3d_reassignment_carrier(&mut heap, &seed);
        let e = s3e_vec_of_tuples_carrier(&mut heap, &seed);

        // Every fixture must have allocated: 3 + 1 + 1 + 3 + 3 conses, plus the
        // tuple/list allocations each one finishes with. The exact figure is
        // asserted rather than a bare `> 0` so that a fixture reduced to a stub
        // by a future edit fails here instead of passing vacuously.
        assert_eq!(heap.cells(), 21, "fixture allocation count changed");

        // Known-answer vector, measured from this code rather than derived: the
        // outputs are arithmetically meaningless, so the only honest assertion
        // about them is that they are exactly what these five shapes produce.
        // A `> 0` or a distinctness check would pass for a stubbed fixture.
        assert_eq!(
            [a.0, b.0, c.0, d.0, e.0],
            [6, 14, 6, 12, 6],
            "a fixture's shape changed"
        );
    }
}
