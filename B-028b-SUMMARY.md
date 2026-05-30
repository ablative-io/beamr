---
phase: B
plan: "028b"
subsystem: native/stdlib
tags: [stdlib, higher-order, beam-bytecode, erlang-compilation]
dependency-graph:
  requires: [B-028a]
  provides: [lists:map/2, lists:foldr/3, lists:foreach/2, maps:map/2-stub]
  affects: [gleam_otp, interpreter]
tech-stack:
  added: [erlc]
  patterns: [compiled-erlang-stdlib, beam-fixture-bundling]
key-files:
  created:
    - crates/beamr/fixtures/stdlib/lists.erl
    - crates/beamr/tests/fixtures/stdlib/lists.beam
    - crates/beamr/tests/stdlib_loading.rs
  modified:
    - crates/beamr/src/native/stdlib_stubs/collection_bifs.rs
    - crates/beamr/src/native/stdlib_stubs/collection_bifs_tests.rs
    - crates/beamr/src/native/stdlib_stubs/mod.rs
    - crates/beamr/src/native/stdlib_stubs/tests.rs
decisions:
  - Higher-order list functions implemented as compiled Erlang bytecode rather than native Rust BIFs because NativeFn cannot re-enter the interpreter to call BEAM closures
  - maps:map/2 left as a stub BIF returning badarg because its Erlang implementation requires maps:to_list/1 which is not yet available within Erlang-level code
  - lists.erl compiled with erlc and bundled as a .beam fixture for loading via --dir
metrics:
  duration: "4 minutes"
  completed: "2026-05-30T20:01:21Z"
  tests-before: 443
  tests-after: 450
---

# Phase B Plan 028b: Higher-Order Stdlib as Compiled BEAM Bytecode Summary

Compiled Erlang source for lists:map/2, lists:foldr/3, lists:reverse/1, lists:foreach/2 into bundled .beam bytecode, plus maps:map/2 stub BIF for import resolution.

## What Was Done

### 1. Erlang Source and Compilation

Created `crates/beamr/fixtures/stdlib/lists.erl` with four exported functions:
- `map/2` — applies a closure to each element of a list
- `foldr/3` — right-fold with accumulator over a list
- `reverse/1` — reverses a proper list (duplicates native stub, harmless)
- `foreach/2` — applies a closure for side effects over a list

Compiled with `erlc` to produce `crates/beamr/tests/fixtures/stdlib/lists.beam` (1044 bytes).

### 2. maps:map/2 Stub BIF

Registered `maps:map/2` as a native BIF stub in `collection_bifs.rs` that returns `badarg`. This is a documented limitation: the function requires interpreter re-entry to call its closure argument, and the Erlang implementation requires `maps:to_list/1` to be available within Erlang code. The stub satisfies import resolution so modules that reference `maps:map/2` can load.

### 3. Integration Tests

Added `crates/beamr/tests/stdlib_loading.rs` with 5 integration tests:
- `stdlib_lists_beam_parses_without_errors` — verifies the compiled .beam fixture decodes
- `stdlib_lists_exports_higher_order_functions` — verifies map/2, foldr/3, reverse/1, foreach/2 exports
- `stdlib_lists_loads_into_module_registry` — verifies full load with zero unresolved imports
- `calling_module_resolves_lists_map_via_loaded_module` — verifies MFA lookup finds all exports
- `maps_map_stub_resolves_in_bif_registry` — verifies maps:map/2 BIF registration

Added 2 unit tests for maps:map/2 stub in `collection_bifs_tests.rs`.

## How It Works

The BEAM module loading path already supports cross-module resolution. When `lists.beam` is loaded into the `ModuleRegistry` before other modules, any module importing `lists:map/2` resolves it via `ResolvedImportTarget::Code` pointing to the label in the lists module. The interpreter then jumps into the lists module's bytecode, which can call closures via `call_fun` opcodes that the interpreter already supports.

In the CLI, users load the stdlib via `--dir`:
```
beamr app.beam --dir path/to/stdlib/
```

## Deviations from Plan

None — plan executed exactly as written.

## Self-Check: PASSED

All 8 files verified present. Commit 495e9ac verified in git log. cargo clippy --workspace -- -D warnings clean. cargo test --workspace: 450 tests passing.
