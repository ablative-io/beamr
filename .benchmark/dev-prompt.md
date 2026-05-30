Implement every R# in this brief. Run cargo check, cargo clippy -- -D warnings, and cargo test on affected crates. Fix any failures before submitting.



## Brief: B-002 — Implement the global atom table

Implement the global atom table in crates/beamr/src/atom/. The Atom type is a newtype over u32 (the index into the table). The AtomTable struct is a concurrent intern map: given a string, it returns the Atom index, inserting if new. Given an Atom index, it returns the string. It must be safe for concurrent reads from all scheduler threads and occasional writes during module loading. Pre-register a fixed set of common atoms at construction so they have stable, known indices. The mod.rs wires the module and re-exports the public API.

## Requirements

### R1: Atom newtype

beamr::atom SHALL define an Atom newtype wrapping a u32 index. Atom SHALL implement Copy, Clone, Eq, PartialEq, Hash, and Debug. Atom SHALL NOT expose its inner index publicly — Atom::index() SHALL be pub(crate), not pub. External crates SHALL compare atoms by equality, not by index value.

Modify: crates/beamr/src/atom/mod.rs

Acceptance:
- Atom is a public struct in beamr::atom
- Atom implements Copy, Clone, Eq, PartialEq, Hash, Debug
- Atom has no public fields (inner index is private)
- Atom::index() is pub(crate) — accessible within beamr but not from beamr-cli or other external crates
- Two Atoms with the same index are equal; two with different indices are not
- Attempting to call atom.index() from beamr-cli produces a compile error

Checklist:
- C12: Global atom table implemented as a concurrent map supporting lock-free reads

### R2: AtomTable concurrent intern map

beamr::atom::table SHALL define an AtomTable struct that supports concurrent interning and lookup. WHEN a new string is interned THE SYSTEM SHALL assign it the next sequential index and store the mapping in both directions. WHEN an already-interned string is interned again THE SYSTEM SHALL return the existing index without creating a duplicate. It SHALL NOT require an exclusive lock for read operations.

Modify: crates/beamr/src/atom/table.rs

Acceptance:
- AtomTable is a public struct in beamr::atom::table
- AtomTable::new() creates an empty table (before pre-registration)
- AtomTable::intern("hello") returns an Atom; calling intern("hello") again returns the same Atom
- AtomTable::intern("hello") and AtomTable::intern("world") return different Atoms
- AtomTable::resolve(atom) returns Some(&str) for a valid atom
- AtomTable::resolve(invalid_atom) returns None for an index that was never interned

Checklist:
- C12: Global atom table implemented as a concurrent map supporting lock-free reads
- C13: Inserting a new atom string returns a unique Atom index
- C14: Inserting an already-interned atom string returns the same index
- C15: Lookup by index returns the original atom string

### R3: Thread-safe concurrent access

WHILE multiple scheduler threads are running THE SYSTEM SHALL allow concurrent intern and resolve operations on the same AtomTable without data races. Concurrent inserts of the same string from different threads SHALL converge on a single index — no duplicates. The implementation SHALL use a lock-free or sharded concurrent map (e.g. dashmap). It SHALL NOT use a single Mutex<HashMap> for the hot read path.

Modify: crates/beamr/src/atom/table.rs crates/beamr/Cargo.toml

Acceptance:
- Spawning 8 threads that each intern the same 100 strings results in exactly 100 unique atoms in the table
- Spawning 8 threads that each intern distinct strings results in all strings present with unique indices
- No Mutex<HashMap> wrapping the primary lookup or insert path (verified by code inspection)

Checklist:
- C16: Concurrent inserts from multiple threads never produce duplicate entries for the same string

### R4: Pre-registered common atoms

AtomTable SHALL provide a with_common_atoms() constructor that pre-registers the following atoms at known, stable indices: ok, error, true, false, nil, undefined, normal, kill, EXIT, badarg, badarith, badmatch, function_clause, case_clause, if_clause, undef, badfun, badarity, noproc. The indices of these atoms SHALL be accessible as associated constants on the Atom type (e.g. Atom::OK, Atom::ERROR). Pre-registered atoms SHALL NOT be re-assignable — their indices are fixed for the lifetime of the table.

Modify: crates/beamr/src/atom/table.rs crates/beamr/src/atom/mod.rs

Acceptance:
- AtomTable::with_common_atoms() returns a table with all 19 listed atoms pre-interned
- Atom::OK, Atom::ERROR, Atom::TRUE, Atom::FALSE, Atom::NIL are public associated constants
- table.resolve(Atom::OK) returns Some("ok")
- table.resolve(Atom::ERROR) returns Some("error")
- table.intern("ok") on a with_common_atoms() table returns Atom::OK (does not create a duplicate)

Checklist:
- C17: Common atoms pre-registered at table creation: ok, error, true, false, nil, undefined, normal, kill, EXIT, badarg, badarith, badmatch, function_clause, case_clause, if_clause, undef, badfun, badarity, noproc

### R5: Module wiring and public API re-exports

crates/beamr/src/atom/mod.rs SHALL re-export the public API: Atom and AtomTable. It SHALL contain only pub mod and pub use declarations — no logic. The atom module SHALL be usable as beamr::atom::Atom and beamr::atom::AtomTable from external crates.

Modify: crates/beamr/src/atom/mod.rs

Acceptance:
- use beamr::atom::Atom compiles from beamr-cli
- use beamr::atom::AtomTable compiles from beamr-cli
- atom/mod.rs contains no fn, struct, enum, trait, or impl blocks

Checklist:
- C12: Global atom table implemented as a concurrent map supporting lock-free reads

### R6: Unit and concurrency tests

WHEN the atom table is tested THE SYSTEM SHALL include tests for: basic intern/resolve round-trip, idempotent interning, common atom constants, concurrent interning from multiple threads, and resolve for invalid indices. Tests SHALL live in a #[cfg(test)] mod tests block within table.rs.

Modify: crates/beamr/src/atom/table.rs

Acceptance:
- cargo test -p beamr atom passes with all tests green
- A test verifies intern("x") followed by resolve returns "x"
- A test verifies intern("x") called twice returns the same Atom
- A test verifies Atom::OK resolves to "ok" after with_common_atoms()
- A test spawns multiple threads doing concurrent inserts and asserts no duplicates
- A test verifies resolve on an out-of-range index returns None

Checklist:
- C13: Inserting a new atom string returns a unique Atom index
- C14: Inserting an already-interned atom string returns the same index
- C15: Lookup by index returns the original atom string
- C16: Concurrent inserts from multiple threads never produce duplicate entries for the same string

## Scout Context

### R1
Files: crates/beamr/src/atom/mod.rs:1-6, crates/beamr/src/atom/table.rs:1-9, docs/design/beamr/briefs/B-002.md:30-43, docs/design/beamr/briefs/B-005.md:68-82
Approach: In `crates/beamr/src/atom/table.rs`, add `#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)] pub struct Atom(u32);` plus an `impl Atom` containing `pub(crate) const fn new(index: u32) -> Self` and `pub(crate) const fn index(self) -> u32`. Do not expose the field or make `index` public. Re-export from `atom/mod.rs` in R5 rather than defining the type there.
Notes: External compile-error acceptance for `atom.index()` is satisfied by `pub(crate)` visibility; a negative compile test would need trybuild or manual verification, but avoid changing beamr-cli just to leave an unused import because clippy -D warnings may fail.

### R2
Files: crates/beamr/src/atom/table.rs:1-9, crates/beamr/Cargo.toml:1-7, docs/design/beamr/DESIGN.md:95-99, docs/adr/002-atom-table-in-core.md:22-39, docs/design/beamr/briefs/B-003.md:53-61
Approach: Implement `pub struct AtomTable` with two directions: a concurrent name-to-Atom map and index-to-string map, plus an `AtomicU32` next index. Use `DashMap` for reads/writes. Because `resolve(&self) -> Option<&str>` cannot safely return a reference into a DashMap guard, store interned strings as leaked `&'static str` in the reverse map; this is compatible with the brief boundary that atoms are never freed. Suggested fields: `by_name: DashMap<&'static str, Atom>`, `by_index: DashMap<u32, &'static str>`, `next_index: AtomicU32`.
Notes: A naive `DashMap<u32, String>` reverse map forces returning a guard-owned reference, not `Option<&str>`. If using `fetch_add` before winning insertion, concurrent duplicates can burn/gap indices; allocate the index only inside the Vacant branch of a `DashMap::entry`-style insertion.

### R3
Files: crates/beamr/src/atom/table.rs:1-9, crates/beamr/Cargo.toml:7, docs/design/beamr/briefs/B-001.md:53-53, docs/design/beamr/briefs/B-001.md:172-172, docs/design/beamr/DESIGN.md:126-131
Approach: Add `dashmap = "6.1.0"` under `crates/beamr/Cargo.toml [dependencies]`. In `intern`, first try a fast `by_name.get(name)` read, then use a sharded-map entry path to make concurrent inserts of the same string converge on one Atom. Use `AtomicU32` for unique sequential index allocation and insert into both maps before returning from `intern`. Tests should use `Arc<AtomTable>` and spawn 8 threads.
Notes: Do not use `Mutex<HashMap>` for primary lookup/insert. `Ordering::Relaxed` is enough for unique counter allocation because DashMap synchronization handles map visibility, but `SeqCst` is acceptable if the implementer wants simpler review optics. The API has no error return for u32 overflow; practical implementation can leave this as out-of-scope unless adding an internal assertion.

### R4
Files: crates/beamr/src/atom/table.rs:1-9, crates/beamr/src/atom/mod.rs:1-6, docs/design/beamr/CHECKLIST.md:17-24, docs/design/beamr/briefs/B-005.md:99-108, docs/design/beamr/briefs/B-007.md:34-69
Approach: Add public associated constants on `Atom` with zero-based stable indices in the checklist order: `OK=0`, `ERROR=1`, `TRUE=2`, `FALSE=3`, `NIL=4`, `UNDEFINED=5`, `NORMAL=6`, `KILL=7`, `EXIT=8`, `BADARG=9`, `BADARITH=10`, `BADMATCH=11`, `FUNCTION_CLAUSE=12`, `CASE_CLAUSE=13`, `IF_CLAUSE=14`, `UNDEF=15`, `BADFUN=16`, `BADARITY=17`, `NOPROC=18`. Define a single `COMMON_ATOMS: &[(&str, Atom)]` table and implement `AtomTable::with_common_atoms()` by inserting that table into both maps, then setting `next_index` to 19.
Notes: Use string `"EXIT"` with uppercase letters but Rust constant `Atom::EXIT`. Zero-based indices match B-003’s convention that BEAM file local atom index 0 is the module name, and no other design source specifies one-based common atom indices.

### R5
Files: crates/beamr/src/atom/mod.rs:1-6, crates/beamr/src/lib.rs:1-19, crates/beamr-cli/Cargo.toml:11-12, crates/beamr-cli/src/main.rs:1-10, docs/design/beamr/DESIGN.md:325-390
Approach: Make `crates/beamr/src/atom/mod.rs` minimal: module docs if desired, then `pub mod table;` and `pub use table::{Atom, AtomTable};`. Keep all `Atom`/`AtomTable` definitions and impls in `table.rs`. This makes both `beamr::atom::Atom` and `beamr::atom::AtomTable` usable from external crates while preserving `beamr::atom::table::AtomTable`.
Notes: Do not leave test imports in beamr-cli for verification; unused imports can become clippy failures. Manual or temporary checks are fine, but final code should keep CLI unchanged unless a later brief requires it.

### R6
Files: crates/beamr/src/atom/table.rs:1-9, docs/design/beamr/briefs/B-002.md:118-153, docs/design/beamr/USER-STORIES.md:43-55, docs/design/beamr/CHECKLIST.md:19-24
Approach: Add unit tests at the bottom of `table.rs`: round-trip `intern("x")` then `resolve`; idempotent `intern("x")` twice; distinct `hello`/`world`; `with_common_atoms()` resolves `Atom::OK`/`ERROR` and `intern("ok") == Atom::OK`; invalid atom using `Atom::new(999_999)` returns None; concurrent same-strings test with `Arc<AtomTable>`, 8 threads, 100 names, and a `HashSet<Atom>` length 100; concurrent distinct-strings test asserting all resolved and unique. Import `std::{collections::HashSet, sync::Arc, thread}` inside tests.
Notes: The `cargo test -p beamr atom` filter should match tests nested under module path `atom::table::tests::*`, so test names need not all include `atom`. Prefer comparing `Atom` equality/HashSet rather than relying on public index access, though crate-local tests can use `Atom::new`/`index` if needed.

## Verification

- cargo check --workspace
- cargo clippy --workspace -- -D warnings
- cargo test -p beamr atom -- --nocapture
- Code inspection: `crates/beamr/src/atom/mod.rs` contains only docs, `pub mod table;`, and `pub use table::{Atom, AtomTable};` — no struct/fn/impl/etc.
- Code inspection: no `Mutex<HashMap>` or single global mutex in AtomTable primary intern/resolve path.
- Optional manual external API check: from beamr-cli or a scratch external crate, `use beamr::atom::{Atom, AtomTable};` compiles, while calling `atom.index()` does not.

## Boundaries

- SHALL NOT implement atom garbage collection or atom table compaction (atoms are never freed — acceptable for our scoped use)
- SHALL NOT implement atom-to-term conversion (B-005 scope — term representation)
- SHALL NOT add the AtomTable as a field on any process or VM struct (wiring is a later brief's responsibility)
- SHALL NOT implement string-to-atom conversion from Gleam bytecode (B-003 scope — loader)

Full design document: docs/design/beamr/DESIGN.md

For each R#, report: status, files changed, how satisfied, any deviation. For each C# and S# assigned to the R#, report whether delivered. Attest: no panics/unwraps in library code, no unsafe, boundaries respected, tests pass.
