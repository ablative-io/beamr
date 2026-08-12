# #103 discriminator — PREDICTION, registered BEFORE the measurement

Written at beamr `99f7e48`, tree clean. The measurement is: make `new()` seat
`COMMON_ATOMS`, then run `cargo test --workspace --all-features` (canon leg 7).

## The question

Do any of the 185 `AtomTable::new()` test sites DEPEND on the table being
empty — i.e. would they change verdict once every interned name is assigned
index 77+ instead of 0,1,2,…?

## Static ground (measured first, at 99f7e48)

* 77 `Atom::*` constants, 77 `COMMON_ATOMS` entries, indices 0..76 contiguous,
  no gaps, no duplicates, `next_index` lands exactly at 77. Seating is
  arithmetically safe.
* 27 raw `Atom::new(N)` sites outside `table.rs`. Every one uses either an
  opaque marker deliberately clear of the constant range (401-406, 450-453,
  999, 999_999) or a constants-range index used as an opaque round-trip token
  (0, 9) whose name is never asserted.
* `Atom::new` is `pub(crate)`, so the 7 integration-test files under
  `crates/beamr/tests/` cannot construct an atom by index at all.
* `scheduler/tests.rs:692` asserts `scheduler.atom_count() == atom_table.len()`
  — RELATIVE, both sides move together, safe.
* `loader/parser.rs:120` (`module.atoms.len() == 7`) and
  `loader/decode/chunks.rs:549` (`atoms.len() == 3`) count a *parsed module's*
  atom chunk, not the intern table. Not affected.
* `AtomTable::default()` has zero reach in the workspace (two-arm probe,
  arm A rc 0 / control rc 101 E0599).

## PREDICTION

**Small, and plausibly zero.** I expect 0-3 failures out of 2110.

The failure shape I CANNOT rule out statically, and am watching for:
1. a test that interns a name into a `new()` table and asserts the result
   equals a specific low index or a specific `Atom::*` constant;
2. a test that asserts `resolve(Atom::SOMETHING) == None` on a `new()` table,
   i.e. uses emptiness as the thing under test;
3. a test asserting an exact atom COUNT after interning N names.

## What each outcome licenses

* **0 failures** ⇒ no test depends on emptiness ⇒ disposition (a) collapses to
  something strictly better than filed: `new()` seats the constants and **no
  empty constructor is needed at all**, so the footgun door closes completely
  rather than being renamed. `with_common_atoms()` becomes a delegating alias
  (440 call sites keep working) or a rename lane of its own.
* **A few failures, all shape 1-3** ⇒ same disposition, plus those named sites
  re-pointed at a private/test-only empty constructor. Each one gets read
  individually — a failure is not automatically a site that WANTS emptiness,
  it may be a site that was silently wrong and is now loudly wrong.
* **Many failures** ⇒ the discriminator has found real dependence and the
  ruling needs Cally/Waffles before any patch.

⛔ A failing test is NOT licence to weaken the assertion to make it pass.
