//! AR-1 row 1 — the control fixtures for `ordering_detector.py`.
//!
//! This module ships no behaviour. It exists so the accumulator-rooting
//! lane's **ordering-sensitive** detector at
//! `docs/design/beamr/briefs/evidence/accumulator-rooting/ordering_detector.py`
//! carries, on every run, one committed site it must FLAG and one it must
//! CLEAR — the gate's row-1 break-it arm and control, held live in the tree
//! rather than constructed once and forgotten.
//!
//! # Why ordering, not presence
//!
//! Row 1's finding: the eleven accumulator crossings **already contain a
//! rooting call** — the terminal `alloc_list`/`alloc_tuple` roots its
//! elements. It roots values that are already stale, and rooting a stale
//! pointer does not recover it. A presence-sensitive instrument passes all
//! eleven while they are still broken. The detector must therefore key on
//! WHERE the rooting call sits relative to the first collecting call on the
//! carrier's live range:
//!
//! * [`fixtures::ord_bad_root_after_collect`] — a rooting call is present
//!   (`with_rooted`, naming the carrier) but sits AFTER a collecting call
//!   made while the carrier is live. **The detector must flag it.** If it
//!   clears, the instrument is measuring presence and certifies nothing.
//! * [`fixtures::ord_good_rooted_before_collect`] — the rooting scope opens
//!   before any collecting call. **The detector must clear it.** If it
//!   flags, "flags everything" is being read as a pass.
//!
//! # Fixture design constraints (all load-bearing)
//!
//! * **Local types only** ([`fixtures::OrdHeap`], [`fixtures::OrdTerm`]):
//!   the detector is syntactic, so these are indistinguishable from the
//!   real allocator to the instrument — and untouchable by any type-level
//!   remedy the lane lands, so the controls survive the lane succeeding.
//! * **Zero `shape_hunt.py` hits, by construction.** That instrument's
//!   S3a–S3e classes key on `Vec` binds, `match`/`if` binds, literal binds,
//!   `let`-less reassignment, and its `ALLOC` spelling list. These fixtures
//!   use none of those shapes and collect via the spelling
//!   `alloc_fixture_term`, which is outside `ALLOC`. Adding a hit here
//!   would put an unruled row in that hunt's ledger reconciliation (gate
//!   R10 as amended: a new unruled hit FAILS).
//! * **Markers are binding names** (`ord_bad_tail`, `ord_good_tail`),
//!   unique in this file, so no comment reflow can strip them and the
//!   detector's uniqueness assert (0 or ≥2 matches = UNUSABLE, never PASS)
//!   has something exact to hold.
//! * **`cargo fmt` is the checked hazard**: the detector reads per-line, so
//!   a reflow that splits a `with_rooted(&[carrier...])` line kills that
//!   fixture's control. The battery runs fmt before the detector, so a
//!   reflow turns the control RED on the next run instead of rotting.
//!
//! # Declared coupling
//!
//! ⚠️ These fixtures work because `ordering_detector.py` reads bind syntax
//! and call spellings per line and is blind to `cfg` and to types. If that
//! detector ever becomes AST-based, these controls must be re-sited. The
//! file must also never be renamed to anything ending `_tests.rs` — the
//! sibling hunt's `source_files()` skip-list convention applies to every
//! instrument in this lane's family.

// Load-bearing for the instrument, exactly as in `ar1_shape_control.rs`:
// the attribute-plus-brace is what the labeller keys on, and it is what
// keeps these fixtures out of the shipped binary while the per-line
// detector still sees them.
#[cfg(test)]
mod fixtures {
    // Covers the lint-shaped remedy arm in one place, as in
    // `ar1_shape_control.rs`: a future rule of any name is caught by one
    // suppression rather than one guard per call.
    #![allow(clippy::disallowed_methods)]

    /// A term-shaped value that is deliberately **not** `crate::term::Term`,
    /// so no type-level remedy can reach these fixtures.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct OrdTerm(usize);

    /// The inert seed value both fixtures start their carrier from.
    const NIL: OrdTerm = OrdTerm(0);

    /// A local allocator carrying the **spellings** the ordering detector
    /// reads — `alloc_fixture_term` (collecting) and `with_rooted`
    /// (rooting) — and none of the real allocator's semantics.
    #[derive(Default)]
    struct OrdHeap {
        cells: usize,
    }

    impl OrdHeap {
        /// The COLLECTING spelling. Matches the detector's `alloc_*` class
        /// and deliberately does NOT match `shape_hunt.py`'s fixed `ALLOC`
        /// list, so this file adds zero hits to that instrument.
        fn alloc_fixture_term(&mut self, seed: usize) -> OrdTerm {
            self.cells += 1;
            OrdTerm(seed ^ self.cells)
        }

        /// The ROOTING spelling, signature-shaped like the real
        /// `ProcessContext::with_rooted`: roots first, closure second.
        fn with_rooted<R>(
            &mut self,
            roots: &[OrdTerm],
            f: impl FnOnce(&mut Self, &[OrdTerm]) -> R,
        ) -> R {
            f(self, roots)
        }

        /// A terminal combiner with a spelling the detector must NOT read
        /// as collecting or rooting — it exists so fixture bodies can end
        /// without adding an event to either class.
        fn finish(&mut self, parts: &[OrdTerm]) -> OrdTerm {
            self.cells += parts.len();
            OrdTerm(parts.iter().fold(self.cells, |acc, p| acc ^ p.0))
        }

        /// Cells handed out, so the `#[test]` below can assert the fixtures
        /// actually ran rather than merely compiled.
        fn cells(&self) -> usize {
            self.cells
        }
    }

    /// **BAD-ORDER** — the row-1 break-it arm, committed. The carrier
    /// `ord_bad_tail` is live across a collecting call, and the rooting
    /// call that names it arrives only afterwards: rooting the stale value.
    /// An ordering-sensitive detector flags this; a presence-sensitive one
    /// clears it, which is exactly the failure row 1 exists to catch.
    fn ord_bad_root_after_collect(heap: &mut OrdHeap) -> OrdTerm {
        let ord_bad_tail = NIL;
        let item = heap.alloc_fixture_term(3);
        heap.with_rooted(&[ord_bad_tail, item], |h, roots| h.finish(roots))
    }

    /// **GOOD-ORDER** — the row-1 clean control, committed. The rooting
    /// scope opens on `ord_good_tail` before any collecting call runs; the
    /// collect happens inside the scope. The detector must clear this, or
    /// "flags everything" reads as a pass.
    fn ord_good_rooted_before_collect(heap: &mut OrdHeap) -> OrdTerm {
        let ord_good_tail = NIL;
        heap.with_rooted(&[ord_good_tail], |h, roots| {
            let item = h.alloc_fixture_term(5);
            h.finish(&[roots[0], item])
        })
    }

    /// Keeps both fixtures **compiled and called**, so one broken by a
    /// refactor fails the build instead of quietly ceasing to be a control.
    /// The assertions are exact known-answer values measured from this
    /// code: a `> 0` would pass for a stubbed fixture.
    #[test]
    fn both_ordering_fixtures_are_live() {
        let mut heap = OrdHeap::default();

        let bad = ord_bad_root_after_collect(&mut heap);
        let good = ord_good_rooted_before_collect(&mut heap);

        // 1 collect + 2 finish-parts, then 1 collect + 2 finish-parts.
        assert_eq!(heap.cells(), 6, "fixture allocation count changed");
        assert_eq!(
            [bad.0, good.0],
            [1, 7],
            "a fixture's shape changed — re-derive the detector controls"
        );
    }
}
