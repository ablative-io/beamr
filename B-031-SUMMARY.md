---
phase: beamr
plan: B-031
subsystem: native/gleam_ffi
tags: [gleam_erlang_ffi, process-utilities, bif, registry, links, supervision]
dependency-graph:
  requires: [B-027, B-030]
  provides: [gleam_erlang_ffi process utilities]
  affects: [beamr-cli registration pipeline]
tech-stack:
  added: []
  patterns: [facility-delegation, gleam-nil-atom-mapping]
key-files:
  created:
    - crates/beamr/src/native/gleam_ffi.rs
    - crates/beamr/src/native/gleam_ffi_tests.rs
  modified:
    - crates/beamr/src/native/mod.rs
    - crates/beamr-cli/src/main.rs
decisions:
  - Separate gleam_ffi.rs from selector_ffi.rs to keep both under 500 lines
  - Use Term::atom(Atom::NIL) for Gleam Nil, not Term::NIL (empty list)
  - Simplified pid_from_dynamic error to {error, nil} instead of full DecodeError list
  - flush_messages/0 is a no-op stub since gleam_otp does not rely on it critically
metrics:
  duration: 211s
  completed: 2026-05-30T20:31:20Z
---

# B-031: gleam_erlang_ffi -- process utilities Summary

Thin facility-delegation BIFs for process utilities under gleam_erlang_ffi, using atom nil for Gleam Nil returns.

## What was implemented

### R1: Process flag and link wrappers
- `trap_exits/1`: Delegates to `LinkFacility::set_trap_exit()`, returns atom nil
- `link/1`: Delegates to `LinkFacility::link()`, returns atom nil
- `demonitor/1`: Delegates to `SupervisionFacility::demonitor()`, returns atom nil

### R2: Sleep, flush, registry wrappers
- `sleep/1`: Uses `std::thread::sleep(Duration::from_millis(ms))`, returns atom nil
- `sleep_forever/0`: Infinite loop with `Duration::MAX`, never returns
- `flush_messages/0`: No-op stub returning atom nil
- `register_process/2`: Delegates to `RegistryFacility::register()`, returns atom nil
- `unregister_process/1`: Delegates to `RegistryFacility::unregister()`, returns atom nil
- `process_named/1`: Delegates to `RegistryFacility::whereis()`, returns `{ok, Pid}` or `{error, nil}`

### R3: pid_from_dynamic/1 and registration
- `pid_from_dynamic/1`: Type-checks term, returns `{ok, Pid}` or `{error, nil}`
- All 10 BIFs registered under `gleam_erlang_ffi` module atom via `register_gleam_ffi_bifs()`
- CLI wired in `load_context()` after `register_selector_bifs()`

## Key design decisions

1. **Separate file**: Created `gleam_ffi.rs` (245 lines) instead of adding to `selector_ffi.rs` (337 lines) to keep both under the 500-line limit.

2. **Gleam Nil mapping**: Gleam's `Nil` compiles to the atom `nil` (index 4), not BEAM's empty list `[]`. All Nil-returning BIFs use `Term::atom(Atom::NIL)` instead of `Term::NIL`.

3. **Tuple allocation**: Uses `ProcessContext::alloc_tuple()` for result tuples (`{ok, Pid}`, `{error, nil}`), consistent with selector_ffi's `error_nil_tuple` pattern.

4. **Error simplification**: `pid_from_dynamic` returns `{error, nil}` instead of the full Gleam `DecodeError` list, matching the brief's recommendation.

## Deviations from Plan

None -- plan executed exactly as written.

## Test coverage

40 unit tests covering:
- All 10 BIFs with happy-path and error-path coverage
- Arity validation for all functions
- Missing facility handling (badarg when no facility configured)
- Registration verification (all MFAs registered, duplicate detection)
- Coexistence with selector BIFs under the same module atom

## Commits

| Hash | Message |
|------|---------|
| 27bfa4c | feat(B-031): gleam_erlang_ffi process utility BIFs |

## Self-Check: PASSED
