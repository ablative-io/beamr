Explore the codebase and gather implementation context for each R# in this brief. You are read-only — do not modify files.

For each R#, find:
- 2-5 key files the implementer should look at (with line ranges)
- Conventions to match (sibling patterns, naming, error handling)
- A concrete implementation approach
- Any gotchas or edge cases the brief might not have considered

The implementing agent has the same tools you do — focus on saving them time, not cataloguing every file. Be concise.



## Brief: B-002 — Implement the global atom table

The atom table is the most foundational data structure in beamr — nothing else works until it does. The loader cannot decode a .beam file without it (bytecode references atoms by index). The interpreter cannot dispatch on atom values. Term comparison for atoms is cheap only because atoms are interned. This brief delivers the complete, thread-safe atom table that all subsequent components depend on.

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

## Boundaries

- SHALL NOT implement atom garbage collection or atom table compaction (atoms are never freed — acceptable for our scoped use)
- SHALL NOT implement atom-to-term conversion (B-005 scope — term representation)
- SHALL NOT add the AtomTable as a field on any process or VM struct (wiring is a later brief's responsibility)
- SHALL NOT implement string-to-atom conversion from Gleam bytecode (B-003 scope — loader)

Full design document: docs/design/beamr/DESIGN.md
