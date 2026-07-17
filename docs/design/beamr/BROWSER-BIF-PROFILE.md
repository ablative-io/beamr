# BROWSER-BIF-PROFILE — the truthful browser contract (WPORT-5 R1)

STATUS: sealed. This document is the profile pillar ruled by the WPORT-5 brief
(`docs/design/beamr/briefs/WPORT-5.json`), governed by the arc's WPORT-5 section
(`docs/design/beamr/WASM-PORT-ARC.md:105-116`). SCOPE, verbatim from the arc:
"Registration must not imply support, and absent services must not degrade into
silent no-ops or accidental misbehaviour."

Every current-state claim below was re-read at the bytes of the WPORT-5 build
head (base `6f1b51b`); "Found @ `6f1b51b`" records the state the build found,
"Shipped @ build head" records what this build ships. NO ROW'S CLASS IS
"registered" — registration nowhere implies support.

## The seal (do not break)

This document is MECHANICALLY SEALED against the real registered surface:
`crates/beamr-wasm/tests/profile_seal.rs` builds the registry through the REAL
wrapper composition (`beamr_wasm::build_wasm_safe_registry`, the `#[doc(hidden)]`
wrapper of the private `register_wasm_safe_bifs`) and asserts EXACT set equality,
both directions, between the registered `(module, function, arity)` set and this
document's machine-readable row keys — plus the exact row count.

Machine-readable row grammar: inside any region delimited by
`<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->` / `<!-- SEAL:END REGISTERED-MFA-TABLE -->`,
every table line beginning `| `` ` `` carries exactly one key of the form
`module:function/arity` in its first cell. Sub-rows (P14) begin `| ↳` and are
NOT keys. Rows outside sealed regions (unregistered surfaces, dynamic seams)
are context, never keys.

If you add, remove, or rename a registered BIF, the seal test fails until this
document agrees. That is the point.

## Taxonomy

Found (behavior classes, checker-corrected ground vocabulary):

- **WORKS** — executes with working cooperative services.
- **BADARG** — facility-absent refusal: catchable `error:badarg` via the native
  raise mapping (`crates/beamr/src/interpreter/opcodes/native_call.rs:205-224`
  at pin). P1/OQ1 RULED: this idiom is KEPT; it is semantically overloaded with
  genuine argument errors, and that conflation is DEFINED per-row by this table
  (distinguishability is static, not dynamic). `ExecError::ServiceUnavailable`
  push-down is recorded future work.
- **SILENT NO-OP** / **FABRICATED** — the VIOLATION classes the arc bans;
  found at the pin, ELIMINATED by R2 (see per-row notes).
- **PROC-ERROR** — explicit process-fatal scheduler error
  (`ExecError::UnsupportedOpcode`), not catchable.
- **APPROX** — defined approximation, documented in the row.

Shipped (the arc's acceptance taxonomy): **supported** /
**supported-with-defined-approximation** / **unsupported** (with the exact
refusal shape named per row).

## Tallies

Found @ `6f1b51b` (row set re-derived at the build head; reconciles with the
checker-corrected ground pack §2.2 — its dominant-class 130 WORKS = 116 WORKS
here + 14 rows this table refines into APPROX with their texts):

| class | count |
|---|---|
| WORKS | 116 |
| APPROX (defined) | 17 |
| BADARG | 51 |
| SILENT NO-OP (violation) | 9 |
| FABRICATED zeros (violation) | 3 |
| PROC-ERROR | 1 |
| **total** | **197** |

Shipped @ build head (WPORT-5 R2 applied — both violation classes are DEAD):

| class | count |
|---|---|
| supported | 126 |
| supported-with-defined-approximation | 17 |
| unsupported (catchable badarg) | 53 |
| unsupported (process-fatal) | 1 |
| **total** | **197** |

Class deltas R2 shipped: `erlang:display/1` + the 8-member sink family
SILENT→supported; `erlang:statistics/1`, `erlang:memory/0`, `erlang:memory/1`
FABRICATED→unsupported-explicit (OQ3); `erlang:spawn/3` BADARG→supported (OQ8,
plain spawn only). Dispatch-path delta (not a row class): export-fun/`apply` to
ANY registered BIF now dispatches instead of `Undef`
(`crates/beamr/src/scheduler/wasm.rs:805`, consumer
`crates/beamr/src/interpreter/opcodes/closures.rs:154-163`).

## 1. Registered surface (sealed)
### Gate 1 — erlang core (21)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `erlang:+/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:-/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:*/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:div/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:rem/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:</2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:>/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:=</2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:>=/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:==/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:/=/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:=:=/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:=/=/2` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:error/1` | Pure | WORKS | supported | — |  |
| `erlang:throw/1` | Pure | WORKS | supported | — |  |
| `erlang:display/1` | ExternalIo | WORKS | supported | — | FOUND (historical, pre-WPORT-5): bare `println!` discarded by the wasm32 stdio backend while returning `true` — the only `println!` call site in `native/`. SHIPPED (R2 item 4): cooperative path routes the formatted term + newline through `write_to_io_sink` (out stream), threaded `println!` byte-identical (`crates/beamr/src/native/bifs.rs:203-222`). Status cell refreshed at the WPORT-7 fold (2026-07-18) — it had lagged the shipped wiring recorded in this note. |
| `erlang:get_module_info/1` | Pure | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg: `module_source_atom` gates on the missing `code_management_facility` (`crates/beamr/src/native/bifs.rs:256-264`). |
| `erlang:get_module_info/2` | Pure | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg: `module_source_atom` gates on the missing `code_management_facility` (`crates/beamr/src/native/bifs.rs:256-264`). |
| `erlang:send_after/3` | Clock | WORKS | supported | — | Real timers since WPORT-3: `NativeServices.timers` carries the unified `Deliver` wheel (`crates/beamr/src/scheduler/wasm.rs:792-810`). |
| `erlang:start_timer/3` | Clock | WORKS | supported | — | Real timers since WPORT-3: `NativeServices.timers` carries the unified `Deliver` wheel (`crates/beamr/src/scheduler/wasm.rs:792-810`). |
| `erlang:cancel_timer/1` | Clock | WORKS | supported | — | Works against the wired wheel. LATENT SHAPE (P11, pinned in `crates/beamr/src/scheduler/wasm_tests.rs`): on a context with NO timer facility the BIF answers atom `false` (`crates/beamr/src/native/bifs.rs:301-320`) — a missing-service answer indistinguishable from "already fired/cancelled". |
<!-- SEAL:END REGISTERED-MFA-TABLE -->

### Gate 1 — code management (6)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `erlang:load_module/2` | ExternalIo | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg: `code_management_facility` is not injected (`crates/beamr/src/native/code_management_bifs.rs:55-69`). |
| `erlang:purge_module/1` | ExternalIo | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg: `code_management_facility` is not injected (`crates/beamr/src/native/code_management_bifs.rs:55-69`). |
| `erlang:delete_module/1` | ExternalIo | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg: `code_management_facility` is not injected (`crates/beamr/src/native/code_management_bifs.rs:55-69`). |
| `erlang:check_old_code/1` | ExternalIo | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg: `code_management_facility` is not injected (`crates/beamr/src/native/code_management_bifs.rs:55-69`). |
| `erlang:check_process_code/2` | ExternalIo | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg: `code_management_facility` is not injected (`crates/beamr/src/native/code_management_bifs.rs:55-69`). |
| `code:all_loaded/0` | Pure | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg: `code_management_facility` is not injected (`crates/beamr/src/native/code_management_bifs.rs:55-69`). |
<!-- SEAL:END REGISTERED-MFA-TABLE -->

### Gate 1 — process dictionary (6)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `erlang:put/2` | ProcessLocal | WORKS | supported | — |  |
| `erlang:get/1` | ProcessLocal | WORKS | supported | — |  |
| `erlang:get/0` | ProcessLocal | WORKS | supported | — |  |
| `erlang:erase/1` | ProcessLocal | WORKS | supported | — |  |
| `erlang:erase/0` | ProcessLocal | WORKS | supported | — |  |
| `erlang:get_keys/1` | ProcessLocal | WORKS | supported | — |  |
<!-- SEAL:END REGISTERED-MFA-TABLE -->

### Gate 1 — ETF codecs (6)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `erlang:term_to_binary/1` | Pure | WORKS | supported | — |  |
| `erlang:term_to_binary/2` | Pure | WORKS | supported | — |  |
| `erlang:term_to_iovec/1` | Pure | WORKS | supported | — |  |
| `erlang:term_to_iovec/2` | Pure | WORKS | supported | — |  |
| `erlang:binary_to_term/1` | Pure | WORKS | supported | — |  |
| `erlang:binary_to_term/2` | Pure | WORKS | supported | — |  |
<!-- SEAL:END REGISTERED-MFA-TABLE -->

### Gate 1 — ETS (23)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `ets:new/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg for the whole `ets` family: creation gates on the missing `ets_facility` and every table-taking BIF gates through `resolve_table` (`crates/beamr/src/native/ets_bifs.rs:975-978`). |
| `ets:insert/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:lookup/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:tab2list/1` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:foldl/3` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:match/1` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:match/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:match/3` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:match_object/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:match_delete/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:select/1` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:select/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:select/3` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:delete/1` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:delete/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:give_away/3` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:member/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:first/1` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:next/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:last/1` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:prev/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:info/1` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
| `ets:info/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — |  |
<!-- SEAL:END REGISTERED-MFA-TABLE -->

### Gate 1 — exception (1)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `erlang:raise/3` | Pure | WORKS | supported | — |  |
<!-- SEAL:END REGISTERED-MFA-TABLE -->

### Gate 1 — process_info (4)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `erlang:process_info/1` | Pure | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg (dominant): `process_info_facility` is not injected. See sub-rows. |
| ↳ any argument | | | | | facility-absent badarg (the full-info form always needs the facility) |
| `erlang:process_info/2` | Pure | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg (dominant): `process_info_facility` is not injected. See sub-rows. |
| ↳ `process_info(self(), priority)` | | | | | answers FACILITY-FREE from the in-hand process (`crates/beamr/src/native/process_info_bifs.rs:214-218`) |
| ↳ any other pid/item | | | | | facility-absent badarg |
| `erlang:group_leader/0` | ProcessLocal | WORKS | supported | — | Answers facility-free from the calling process (`crates/beamr/src/native/process_info_bifs.rs:333-338`). |
| `erlang:group_leader/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg (`crates/beamr/src/native/process_info_bifs.rs:340-352`). |
<!-- SEAL:END REGISTERED-MFA-TABLE -->

### Gate 1 — system_info (7)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `erlang:system_info/1` | Pure | BADARG | unsupported (catchable badarg) | — | GENUINE SPLIT (dominant class badarg). See sub-rows; not touched by R2 (facility-free items keep answering). |
| ↳ `wordsize` / `otp_release` / `version` / `system_architecture` | | | | | answer FACILITY-FREE (`crates/beamr/src/native/system_info_bifs.rs:126-129`) |
| ↳ `schedulers` / `process_count` / `process_limit` / `atom_count` / `atom_limit` | | | | | facility-absent badarg via `facility_small_int` (`crates/beamr/src/native/system_info_bifs.rs:239-249`) |
| ↳ any other item | | | | | badarg (unknown item) |
| `erlang:statistics/1` | Pure | FABRICATED (violation) | unsupported (catchable badarg) | — | FOUND: fabricated all-zero summary presented as genuine data via `.unwrap_or_default()` on the missing facility (pin `2cfd6cf` system_info_bifs.rs:142-145). SHIPPED (R2 item 5, OQ3 ruled): catchable facility-absent badarg (`crates/beamr/src/native/system_info_bifs.rs:137-158`); facility-backed builds unchanged. |
| `erlang:memory/0` | Pure | FABRICATED (violation) | unsupported (catchable badarg) | — | FOUND: fabricated zero proplist (pin system_info_bifs.rs:255-260). SHIPPED (R2 item 5, OQ3): facility-absent badarg via `memory_summary` (`crates/beamr/src/native/system_info_bifs.rs:166-187`, `:263-272`). |
| `erlang:memory/1` | Pure | FABRICATED (violation) | unsupported (catchable badarg) | — | FOUND: fabricated zero item. SHIPPED (R2 item 5, OQ3): facility-absent badarg (`crates/beamr/src/native/system_info_bifs.rs:190-205`, `:263-272`). |
| `erlang:ports/0` | Pure | WORKS | supported | — | Returns `[]` — truthful on a VM with no port subsystem (`crates/beamr/src/native/system_info_bifs.rs:207-213`). |
| `erlang:port_info/1` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: returns atom `undefined` for ANY argument (no port metadata exists; success-shaped documented stub, `crates/beamr/src/native/system_info_bifs.rs:215-221`). |
| `erlang:open_port/2` | Pure | BADARG | unsupported (catchable badarg) | — | Deliberately unsupported: explicit badarg (`crates/beamr/src/native/system_info_bifs.rs:223-229`). |
<!-- SEAL:END REGISTERED-MFA-TABLE -->

### Gate 2 — process lifecycle (17)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `erlang:self/0` | Pure | WORKS | supported | yes — see §4 |  |
| `erlang:spawn/3` | Spawn | BADARG | supported | — | FOUND: facility-absent badarg (no spawn facility injected at `2cfd6cf`). SHIPPED (R2 item 2, OQ8 ruled: plain spawn only): a fresh pid whose process runs the named exported function under the cooperative scheduler — owned-arg capture at the facility, deferred MFA-spawn record, post-slice materialization through `module_registry.lookup_mfa` (`crates/beamr/src/scheduler/wasm_native.rs:98-135`, `:563-611`). A failed materialization (e.g. unloaded module) records the CHILD's exit error, observable via `take_exit_error` — the caller keeps the pid, BEAM-adjacent. |
| `erlang:spawn/4` | Spawn | BADARG | unsupported (catchable badarg) | — | STAYS unsupported — WITH A RECORDED BRIEF-VS-BYTES FLAG: `spawn/4` is `spawn(Node, M, F, A)`, registered to `remote_spawn_impl`, which gates on the missing `remote_spawn_facility` and constructs EXTERNAL pids (`crates/beamr/src/native/process_bifs/mod.rs:367-403`). The WPORT-5 brief's headline "plain spawn/3,4" over-names the local surface at the bytes; wiring a remote-spawn facility is outside R2's consume-existing-seams law. See the build report deviation register. |
| `erlang:spawn_link/3` | Spawn | BADARG | unsupported (catchable badarg) | — | Explicit refusal (catchable badarg) citing WPORT-4 tear Ruling 7: cooperative bytecode exits perform NO link propagation, so wiring any link/monitor-bearing spawn would convert that latent gap into reachable silent wrong behaviour. The injected facility refuses these variants (`crates/beamr/src/scheduler/wasm_native.rs:106-114`, `:157-205`); the linking family belongs to the future bytecode-linking brief. |
| `erlang:spawn_link/4` | Spawn | BADARG | unsupported (catchable badarg) | — | Explicit refusal (catchable badarg) citing WPORT-4 tear Ruling 7: cooperative bytecode exits perform NO link propagation, so wiring any link/monitor-bearing spawn would convert that latent gap into reachable silent wrong behaviour. The injected facility refuses these variants (`crates/beamr/src/scheduler/wasm_native.rs:106-114`, `:157-205`); the linking family belongs to the future bytecode-linking brief. |
| `erlang:spawn_monitor/1` | Spawn | BADARG | unsupported (catchable badarg) | — | Explicit refusal (catchable badarg) citing WPORT-4 tear Ruling 7: cooperative bytecode exits perform NO link propagation, so wiring any link/monitor-bearing spawn would convert that latent gap into reachable silent wrong behaviour. The injected facility refuses these variants (`crates/beamr/src/scheduler/wasm_native.rs:106-114`, `:157-205`); the linking family belongs to the future bytecode-linking brief. |
| `erlang:spawn_monitor/3` | Spawn | BADARG | unsupported (catchable badarg) | — | Explicit refusal (catchable badarg) citing WPORT-4 tear Ruling 7: cooperative bytecode exits perform NO link propagation, so wiring any link/monitor-bearing spawn would convert that latent gap into reachable silent wrong behaviour. The injected facility refuses these variants (`crates/beamr/src/scheduler/wasm_native.rs:106-114`, `:157-205`); the linking family belongs to the future bytecode-linking brief. |
| `erlang:spawn_monitor/4` | Spawn | BADARG | unsupported (catchable badarg) | — | Explicit refusal (catchable badarg) citing WPORT-4 tear Ruling 7: cooperative bytecode exits perform NO link propagation, so wiring any link/monitor-bearing spawn would convert that latent gap into reachable silent wrong behaviour. The injected facility refuses these variants (`crates/beamr/src/scheduler/wasm_native.rs:106-114`, `:157-205`); the linking family belongs to the future bytecode-linking brief. |
| `erlang:spawn_opt/2` | Spawn | BADARG | unsupported (catchable badarg) | — | Explicit refusal (catchable badarg) citing WPORT-4 tear Ruling 7: cooperative bytecode exits perform NO link propagation, so wiring any link/monitor-bearing spawn would convert that latent gap into reachable silent wrong behaviour. The injected facility refuses these variants (`crates/beamr/src/scheduler/wasm_native.rs:106-114`, `:157-205`); the linking family belongs to the future bytecode-linking brief. |
| `erlang:spawn_opt/4` | Spawn | BADARG | unsupported (catchable badarg) | — | Explicit refusal (catchable badarg) citing WPORT-4 tear Ruling 7: cooperative bytecode exits perform NO link propagation, so wiring any link/monitor-bearing spawn would convert that latent gap into reachable silent wrong behaviour. The injected facility refuses these variants (`crates/beamr/src/scheduler/wasm_native.rs:106-114`, `:157-205`); the linking family belongs to the future bytecode-linking brief. |
| `erlang:link/1` | ProcessLocal | BADARG | unsupported (catchable badarg) | — | Split — see sub-rows. Dominant: facility-absent badarg (link facility not injected; WPORT-4 Ruling 7 family). |
| ↳ self | | | | | returns `true` FACILITY-FREE (`crates/beamr/src/native/process_bifs/mod.rs:123-126`) |
| ↳ local non-self pid | | | | | facility-absent badarg |
| ↳ remote pid under not(net) | | | | | explicit `noproc` (`crates/beamr/src/native/process_bifs/mod.rs:146`) |
| `erlang:unlink/1` | ProcessLocal | APPROX | supported-with-defined-approximation | — | Split — see sub-rows. Dominant class carried from the checker-corrected ground: documented approximation (the remote arm answers success-shaped `true`). |
| ↳ self | | | | | returns `true` FACILITY-FREE (`crates/beamr/src/native/process_bifs/mod.rs:158-161`) |
| ↳ local non-self pid | | | | | facility-absent badarg |
| ↳ remote pid under not(net) | | | | | SILENT `Ok(true)` (`crates/beamr/src/native/process_bifs/mod.rs:180`) — low reach: Send to a remote pid raises `NoConnection` first (`crates/beamr/src/interpreter/opcodes/messaging.rs:44-45`) |
| `erlang:process_flag/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — | Split — see sub-rows. Dominant: facility-absent badarg. |
| ↳ (`trap_exit`, _) | | | | | badarg on the missing link facility (`crates/beamr/src/native/process_bifs/mod.rs:184-205`) |
| ↳ (`priority`, P) | | | | | WORKS process-locally (same range) |
| `erlang:monitor/2` | ProcessLocal | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg (`supervision_facility` not injected; WPORT-4 Ruling 7 family). |
| `erlang:demonitor/1` | ProcessLocal | BADARG | unsupported (catchable badarg) | — | Facility-absent badarg (`supervision_facility` not injected; WPORT-4 Ruling 7 family). |
| `erlang:exit/1` | ProcessLocal | WORKS | supported | — | Raises FACILITY-FREE and works (`crates/beamr/src/native/process_bifs/mod.rs:286-299`). |
| `erlang:exit/2` | ProcessLocal | APPROX | supported-with-defined-approximation | — | Split — see sub-rows. Dominant class carried from the checker-corrected ground: documented approximation (remote arm success-shaped). |
| ↳ local pid | | | | | facility-absent badarg (supervision facility not injected) |
| ↳ remote pid under not(net) | | | | | `Ok(true)` while undeliverable (`crates/beamr/src/native/process_bifs/mod.rs:314` region; wire-spec ruling 7 best-effort) |
<!-- SEAL:END REGISTERED-MFA-TABLE -->

### Stdlib stubs (106)

<!-- SEAL:BEGIN REGISTERED-MFA-TABLE -->
| MFA | Capability | Found @ `6f1b51b` | Shipped @ build head | Guard-legal | Notes |
|---|---|---|---|---|---|
| `erlang:binary_to_float/1` | Pure | WORKS | supported | — |  |
| `erlang:binary_to_integer/1` | Pure | WORKS | supported | — |  |
| `erlang:binary_to_integer/2` | Pure | WORKS | supported | — |  |
| `erlang:float/1` | Pure | WORKS | supported | — |  |
| `erlang:integer_to_binary/1` | Pure | WORKS | supported | — |  |
| `erlang:integer_to_binary/2` | Pure | WORKS | supported | — |  |
| `erlang:integer_to_list/1` | Pure | WORKS | supported | — |  |
| `erlang:integer_to_list/2` | Pure | WORKS | supported | — |  |
| `erlang:iolist_to_binary/1` | Pure | WORKS | supported | — |  |
| `erlang:list_to_bitstring/1` | Pure | WORKS | supported | — |  |
| `erlang:list_to_tuple/1` | Pure | WORKS | supported | — |  |
| `erlang:tuple_to_list/1` | Pure | WORKS | supported | — |  |
| `erlang:band/2` | Pure | WORKS | supported | — |  |
| `erlang:bnot/1` | Pure | WORKS | supported | — |  |
| `erlang:bor/2` | Pure | WORKS | supported | — |  |
| `erlang:bsl/2` | Pure | WORKS | supported | — |  |
| `erlang:bsr/2` | Pure | WORKS | supported | — |  |
| `erlang:bxor/2` | Pure | WORKS | supported | — |  |
| `math:ceil/1` | Pure | WORKS | supported | — |  |
| `math:floor/1` | Pure | WORKS | supported | — |  |
| `math:exp/1` | Pure | WORKS | supported | — |  |
| `math:log/1` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: non-positive input badargs where OTP raises `badarith` (`crates/beamr/src/native/stdlib_stubs/math_bifs.rs:29-38`). |
| `math:pow/2` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: no overflow check — the `powf` result (possibly ±inf) is returned as a float where OTP raises `badarith` on overflow (`crates/beamr/src/native/stdlib_stubs/math_bifs.rs:40-48`). |
| `rand:uniform/0` | Entropy | APPROX | supported-with-defined-approximation | — | APPROXIMATION: non-seedable host entropy — getrandom `wasm_js` backend (`crates/beamr/Cargo.toml` getrandom feature); no `rand:seed` counterpart exists, so sequences are not reproducible (`crates/beamr/src/native/stdlib_stubs/misc_bifs.rs:136-142`). |
| `logger:warning/2` | ExternalIo | SILENT NO-OP (violation) | supported | — | FOUND: silent no-op — `write_to_io_sink` was an unconditional not(threads) no-op (pin context/mod.rs:1484-1485); returned success while output vanished. SHIPPED (R2 item 4, OQ2 ruled): lands in the registered host sink, console default (`console.log`), out stream; PUSH-ONLY, no flush timer (`crates/beamr/src/native/context/mod.rs:1475-1488`; `crates/beamr-wasm/src/io_sink.rs`). Format is a coarse `[warning] <format> <args>` line (`crates/beamr/src/native/stdlib_stubs/misc_bifs.rs:14-28`). |
| `unicode:characters_to_list/1` | Pure | WORKS | supported | — |  |
| `unicode:characters_to_binary/1` | Pure | WORKS | supported | — |  |
| `sys:debug_options/1` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: no-op stub returning `[]` for any list argument (`crates/beamr/src/native/stdlib_stubs/misc_bifs.rs:166-172`). |
| `gleam_stdlib:print/1` | ExternalIo | SILENT NO-OP (violation) | supported | — | FOUND: silent no-op — `write_to_io_sink` was an unconditional not(threads) no-op (pin context/mod.rs:1484-1485); returned success while output vanished. SHIPPED (R2 item 4, OQ2 ruled): lands in the registered host sink, console default (`console.log`), out stream; PUSH-ONLY, no flush timer (`crates/beamr/src/native/context/mod.rs:1475-1488`; `crates/beamr-wasm/src/io_sink.rs`). |
| `gleam_stdlib:print_error/1` | ExternalIo | SILENT NO-OP (violation) | supported | — | FOUND: silent no-op AND stdout/stderr conflation (pin gleam_stdlib_ffi2.rs:20-24/:30-34 wrote to the shared sink). SHIPPED (R2 item 4): lands in the host sink with the ERR stream tag (`console.error` default) via `write_to_io_sink_tagged` (`crates/beamr/src/native/stdlib_stubs/gleam_stdlib_ffi2.rs:17-53`). |
| `gleam_stdlib:println/1` | ExternalIo | SILENT NO-OP (violation) | supported | — | FOUND: silent no-op — `write_to_io_sink` was an unconditional not(threads) no-op (pin context/mod.rs:1484-1485); returned success while output vanished. SHIPPED (R2 item 4, OQ2 ruled): lands in the registered host sink, console default (`console.log`), out stream; PUSH-ONLY, no flush timer (`crates/beamr/src/native/context/mod.rs:1475-1488`; `crates/beamr-wasm/src/io_sink.rs`). |
| `gleam_stdlib:println_error/1` | ExternalIo | SILENT NO-OP (violation) | supported | — | FOUND: silent no-op AND stdout/stderr conflation (pin gleam_stdlib_ffi2.rs:20-24/:30-34 wrote to the shared sink). SHIPPED (R2 item 4): lands in the host sink with the ERR stream tag (`console.error` default) via `write_to_io_sink_tagged` (`crates/beamr/src/native/stdlib_stubs/gleam_stdlib_ffi2.rs:17-53`). |
| `uri_string:parse/1` | Pure | WORKS | supported | — |  |
| `uri_string:dissect_query/1` | Pure | WORKS | supported | — |  |
| `string:length/1` | Pure | WORKS | supported | — |  |
| `string:reverse/1` | Pure | WORKS | supported | — |  |
| `string:lowercase/1` | Pure | WORKS | supported | — |  |
| `string:uppercase/1` | Pure | WORKS | supported | — |  |
| `string:trim/2` | Pure | WORKS | supported | — |  |
| `string:split/3` | Pure | WORKS | supported | — |  |
| `string:find/2` | Pure | WORKS | supported | — |  |
| `string:next_grapheme/1` | Pure | WORKS | supported | — |  |
| `string:pad/4` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: returns a FLAT BINARY where OTP returns deep chardata/iolist (`crates/beamr/src/native/stdlib_stubs/string_bifs.rs:138`). |
| `string:replace/4` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: flat binary vs OTP iolist shape (`crates/beamr/src/native/stdlib_stubs/string_bifs.rs:169`). |
| `string:slice/3` | Pure | WORKS | supported | — |  |
| `string:equal/2` | Pure | WORKS | supported | — |  |
| `string:is_empty/1` | Pure | WORKS | supported | — |  |
| `binary:part/3` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: no negative-Length form — OTP accepts negative Len to slice backwards; here it badargs (`crates/beamr/src/native/stdlib_stubs/misc_bifs.rs:114-133`). |
| `binary:encode_hex/1` | Pure | WORKS | supported | — |  |
| `binary:decode_hex/1` | Pure | WORKS | supported | — |  |
| `base64:encode/2` | Pure | WORKS | supported | — |  |
| `base64:decode/1` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: strictly canonical input only — OTP tolerates some non-canonical forms (`crates/beamr/src/native/stdlib_stubs/encoding_bifs.rs:91-97`). |
| `io:put_chars/1` | ExternalIo | SILENT NO-OP (violation) | supported | — | FOUND: silent no-op — `write_to_io_sink` was an unconditional not(threads) no-op (pin context/mod.rs:1484-1485); returned success while output vanished. SHIPPED (R2 item 4, OQ2 ruled): lands in the registered host sink, console default (`console.log`), out stream; PUSH-ONLY, no flush timer (`crates/beamr/src/native/context/mod.rs:1475-1488`; `crates/beamr-wasm/src/io_sink.rs`). |
| `io:put_chars/2` | ExternalIo | SILENT NO-OP (violation) | supported | — | FOUND: silent no-op — `write_to_io_sink` was an unconditional not(threads) no-op (pin context/mod.rs:1484-1485); returned success while output vanished. SHIPPED (R2 item 4, OQ2 ruled): lands in the registered host sink, console default (`console.log`), out stream; PUSH-ONLY, no flush timer (`crates/beamr/src/native/context/mod.rs:1475-1488`; `crates/beamr-wasm/src/io_sink.rs`). |
| `io:format/2` | ExternalIo | BADARG | unsupported (catchable badarg) | — | Explicit-unsupported, UNCHANGED BY DESIGN (P3): fails `context.io_message_facility().ok_or_else(badarg)` BEFORE any send or suspend (`crates/beamr/src/native/stdlib_stubs/io_bifs.rs:36-44`, gate at `:215`). No `IoMessageFacility` may be wired naively: every wasm process is its own group leader (`crates/beamr/src/scheduler/wasm.rs:322`, `:364`), so an io_request would self-deadlock (hazard H3). The format/2-vs-format/3 asymmetry is hereby the DEFINED line: sink-based output works; io-protocol IO is unsupported-explicit; a real IO server is recorded future work. |
| `io:format/3` | ExternalIo | SILENT NO-OP (violation) | supported | — | FOUND: silent no-op — `write_to_io_sink` was an unconditional not(threads) no-op (pin context/mod.rs:1484-1485); returned success while output vanished. SHIPPED (R2 item 4, OQ2 ruled): lands in the registered host sink, console default (`console.log`), out stream; PUSH-ONLY, no flush timer (`crates/beamr/src/native/context/mod.rs:1475-1488`; `crates/beamr-wasm/src/io_sink.rs`). |
| `io:get_line/1` | ExternalIo | BADARG | unsupported (catchable badarg) | — | Explicit-unsupported (same io-protocol gate as `io:format/2`; `crates/beamr/src/native/stdlib_stubs/io_bifs.rs:46-55`, suspend path `:223` unreachable without the facility). |
| `io:setopts/2` | ProcessLocal | APPROX | supported-with-defined-approximation | — | APPROXIMATION (carried WORKS-as-defined-stub, tear did not reclassify): success-returning no-op — accepts e.g. `[{encoding, unicode}]` and changes nothing (`crates/beamr/src/native/stdlib_stubs/io_bifs.rs:57-63`). Capability corrected ExternalIo→ProcessLocal (R2 item 6). |
| `io_lib:format/2` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: returns a BINARY where OTP returns a charlist — shape deviation surfaces far from the call site; only the `~s ~p ~w ~n ~~` directive subset is supported and unknown directives badarg explicitly (`crates/beamr/src/native/stdlib_stubs/io_bifs.rs:65-71`, directives `:73-108`). Capability corrected ExternalIo→Pure (R2 item 6). |
| `init:stop/1` | ExternalIo | APPROX | supported-with-defined-approximation | — | APPROXIMATION (P13): exits the CALLING PROCESS only — `request_shutdown` maps to `InstructionOutcome::Exit(ExitReason::Normal)` (`crates/beamr/src/interpreter/opcodes/native_call.rs:253-255`); no VM-wide shutdown exists on the wasm scheduler; the validated exit code is DISCARDED. Defined at the site comment `crates/beamr/src/native/stdlib_stubs/misc_bifs.rs:145-163` (R2 item 8). |
| `maps:from_list/1` | Pure | WORKS | supported | — |  |
| `maps:merge/2` | Pure | WORKS | supported | — |  |
| `maps:remove/2` | Pure | WORKS | supported | — |  |
| `maps:map/2` | Pure | WORKS | supported | — |  |
| `maps:put/3` | Pure | WORKS | supported | — |  |
| `maps:find/2` | Pure | WORKS | supported | — |  |
| `maps:get/2` | Pure | WORKS | supported | — |  |
| `maps:get/3` | Pure | WORKS | supported | — |  |
| `maps:keys/1` | Pure | WORKS | supported | — |  |
| `maps:values/1` | Pure | WORKS | supported | — |  |
| `maps:to_list/1` | Pure | WORKS | supported | — |  |
| `maps:fold/3` | Pure | WORKS | supported | — |  |
| `maps:filter/2` | Pure | WORKS | supported | — |  |
| `maps:merge_with/3` | Pure | WORKS | supported | — |  |
| `maps:update_with/4` | Pure | WORKS | supported | — |  |
| `maps:with/2` | Pure | WORKS | supported | — |  |
| `maps:without/2` | Pure | WORKS | supported | — |  |
| `lists:reverse/1` | Pure | WORKS | supported | — |  |
| `lists:append/1` | Pure | WORKS | supported | — |  |
| `lists:append/2` | Pure | WORKS | supported | — |  |
| `lists:join/2` | Pure | WORKS | supported | — |  |
| `lists:nth/2` | Pure | WORKS | supported | — |  |
| `lists:member/2` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: uses numeric `==` where OTP uses `=:=` — `lists:member(1, [1.0])` is `true` here, `false` in OTP (`crates/beamr/src/native/stdlib_stubs/lists_bifs.rs:76-85`, `numeric_eq` at `:82`). |
| `lists:keyfind/3` | Pure | WORKS | supported | — |  |
| `lists:last/1` | Pure | WORKS | supported | — |  |
| `lists:sort/1` | Pure | WORKS | supported | — |  |
| `lists:flatten/1` | Pure | WORKS | supported | — |  |
| `lists:zip/2` | Pure | WORKS | supported | — |  |
| `lists:unzip/1` | Pure | WORKS | supported | — |  |
| `lists:filter/2` | Pure | WORKS | supported | — |  |
| `lists:filtermap/2` | Pure | WORKS | supported | — |  |
| `lists:map/2` | Pure | WORKS | supported | — |  |
| `lists:reverse/2` | Pure | WORKS | supported | — |  |
| `lists:seq/2` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION: `From > To` returns `[]` where OTP raises `function_clause` (`crates/beamr/src/native/stdlib_stubs/lists_bifs.rs:227-243`, early return at `:233-235`). |
| `lists:keystore/4` | Pure | WORKS | supported | — |  |
| `lists:keysort/2` | Pure | WORKS | supported | — |  |
| `lists:keydelete/3` | Pure | WORKS | supported | — |  |
| `lists:foreach/2` | Pure | WORKS | supported | — |  |
| `timer:sleep/1` | Clock | PROC-ERROR | unsupported (process-fatal `UnsupportedOpcode`) | — | UNSUPPORTED with explicit PROCESS-FATAL error (P7): the only dirty registration on the browser surface — dirty dispatch returns `InstructionOutcome::DirtyCall` BEFORE the native body runs (`crates/beamr/src/interpreter/opcodes/native_call.rs:96-104`) and the cooperative scheduler maps it to `ExecError::UnsupportedOpcode { name: "dirty native call on wasm" }` (`crates/beamr/src/scheduler/wasm.rs:636-646`). NOT a catchable badarg — a scheduler-level process error, JS-observable via `take_exit_error` since R2 item 7. The `std::thread::sleep` body is unreachable on wasm. Reroute-as-yielding-sleep is recorded future work. |
| `json:decode/1` | Pure | WORKS | supported | — |  |
| `json:encode/1` | Pure | WORKS | supported | — |  |
| `json:encode_integer/1` | Pure | WORKS | supported | — |  |
| `json:encode_float/1` | Pure | WORKS | supported | — |  |
| `json:encode_binary/1` | Pure | WORKS | supported | — |  |
| `erlang:fun_info/2` | Pure | APPROX | supported-with-defined-approximation | — | APPROXIMATION — WRONG DATA AS SUCCESS, the worst approximation in this table: `module`/`name`/`type` return the ITEM NAME ITSELF as a binary and `env` is always `[]` (`crates/beamr/src/native/stdlib_stubs/misc_bifs.rs:175-192`, fabrication at `:187`). Only `arity` is real. |
| `io_lib_format:fwrite_g/1` | Pure | WORKS | supported | — |  |
<!-- SEAL:END REGISTERED-MFA-TABLE -->
## 2. Dynamic seams (P15) — classified, not sealed keys

Two open-ended registration seams exist beyond the 197 static rows:

- **`wasm_ffi:js_callback/N`** (`crates/beamr-wasm/src/lib.rs`,
  `register_js_callback_nif`): unregistered callback NAME at call time →
  `Err(undef)`; marshal failure or synchronous JS throw → catchable badarg;
  Promise rejection → error completion, the process exits `ExitReason::Error`.
- **Host async NIFs** (`register_async_nif`): unregistered MFA at call time →
  `Err(undef)` — fails closed.

Bridge execution proof: WPORT-4's native-completion wall pair (WPORT-4 tear
Ruling 4) is the first bridge execution evidence; not duplicated here.

Duplicate-MFA shadowing is IMPOSSIBLE (positive invariant): registration
rejects duplicates with `NativeRegistrationError::DuplicateMfa`
(`crates/beamr/src/native/mod.rs:258-268`), so no host registration can
silently override a stub, and no stub can shadow another.

## 3. Unregistered surface (P12) — the `Undef` context set

Calls into anything not in §1 resolve to no target: load-time
unresolved-import report + call-time `ExecError::Undef` — the ACCEPTED
unsupported shape, and since R2 item 7 it is JS-observable WITH the MFA via
`take_exit_error`/the completion `reason` field.

`Undef` has THREE producers, all surfacing the same shape:

1. `Unresolved` import (never registered);
2. `Denied` — load-time capability policy (`crates/beamr/src/module.rs:40-43`
   at pin);
3. `Deferred` whose module is still unloaded at call time
   (`crates/beamr/src/interpreter/opcodes/core.rs:561-573` at pin) — a call
   into a never-shipped BEAM module surfaces identically.

The five CLI-only registration surfaces NOT registered in the browser
(`crates/beamr-cli/src/main.rs:353-361` at pin), plus the cfg-compiled-out
net/fs families (`crates/beamr/src/native/bifs.rs:59-81`):

1. **gate3** — exactly 59 entries (`erlang:element/2`, `erlang:send/2`,
   `make_ref/0`, `register/2`, `whereis/1`, time functions, type conversions,
   `is_process_alive/1`, `node/0`, …).
2. **register_selector_bifs**
3. **register_gleam_ffi_bifs**
4. **register_meridian_ffi** (includes dirty-marked blocking-IO NIFs)
5. **otp_stubs**

**gate3 MUST-FIX-BEFORE-REGISTER preconditions** (recorded, binding on any
future brief that registers gate3 for the browser): (a) gate3 `erlang:send/2`
SILENTLY DROPS local non-self sends
(`crates/beamr/src/native/gate3_bifs/mod.rs:188-213` at pin, drop documented
at `:191-193`); (b) `is_process_alive/1` returns `false` without a supervision
facility (`crates/beamr/src/native/gate3_bifs/mod.rs:617-620` at pin) — both
success-shaped wrong answers of exactly the class this profile exists to ban.

## 4. Guard-position semantics (P9) — DEFINED behavior, no code change

Guard-BIF dispatch builds a context carrying ONLY the atom table — not even
the now-populated timers (`crates/beamr/src/interpreter/opcodes/guards.rs:202-207`
at pin) — and ANY BIF error in guard position becomes a fail-label jump, not
an exception (`guards.rs:215-222` at pin). Therefore: a missing-facility
badarg in guard position is SILENT FAIL-BRANCH SELECTION. This is standard
BEAM guard semantics and is accepted as DEFINED; the "guard-legal" column in
§1 marks every guard-legal row (the 13 comparison/arithmetic operators and
`erlang:self/0`). Note `erlang:self/0` in guard position on this build: the
guard context carries no pid, so it badargs → silent fail-branch — exactly the
behavior this section defines. A non-Native guard import fails as
`ExecError::InvalidOperand("guard bif native import")` (`guards.rs:193-195`
at pin) — a different shape from the call-path `Undef`.

## 5. Hazards (recorded, with citations)

- **H1 — capability denial is value-shaped**: a denied capability returns
  `{error, capability_denied}` as a NORMAL VALUE, not an exception
  (`crates/beamr/src/interpreter/opcodes/native_call.rs:79-90`, tuple at
  `:266-276` region at pin), and the fallback violation handler writes to
  STDERR — which goes nowhere on wasm32 (`native_call.rs:86` at pin). Inert
  today (browser keeps `CapabilitySet::all()`,
  `crates/beamr/src/scheduler/wasm.rs:321`, `:363`; native roots
  `crates/beamr/src/scheduler/wasm_native.rs:243`, `:588`); BOTH hazards are
  RECORDED VERBATIM as preconditions of the future capability brief. R2 item 6
  fixed the two pure-function misregistrations (`io:setopts/2`,
  `io_lib:format/2`) so that brief starts hygienic.
- **H2 — the badarg conflation** (P1/OQ1): facility-absent refusal and genuine
  argument error share `error:badarg`. DEFINED per-row here; the
  `ServiceUnavailable` push-down is recorded future work.
- **H3 — group-leader self-loop**: every wasm-spawned process is its own group
  leader (`crates/beamr/src/scheduler/wasm.rs:322`, `:364`; native roots
  `crates/beamr/src/scheduler/wasm_native.rs:244`, `:538`, `:589`). Naive
  `IoMessageFacility` wiring = guaranteed self-deadlock for `io:format/2` /
  `io:get_line/1`. FORBIDDEN; the IO-server design is a recorded future brief.
  No `IoMessageFacility` is constructed anywhere in the wasm closure
  (grep-verified on the final diff).
- **H4 — duplicate-MFA impossibility** (positive invariant): see §2.
- **H5 — `cancel_timer` bare-context `false`** (P11): pinned as a unit test so
  a wiring regression cannot silently re-introduce the missing-service answer.
- **H6 — sink classification law** (P11): the captured-sink wall red-lines any
  future `write_to_io_sink` caller that is not classified in this table.

## 6. Recorded future work (the register, carried verbatim to the handoff)

1. `ExecError::ServiceUnavailable`/`notsup` vocabulary push-down into the
   BIF/facility layer (P1/OQ1 — wrong brief here; registry-wide).
2. The IO-server brief (real `IoMessageFacility` + group-leader protocol;
   H3 preconditions bind).
3. The capability brief (H1 preconditions verbatim: value-shaped denial +
   wasm32-dead stderr handler must be fixed before any restriction).
4. `timer:sleep/1` reroute through the WPORT-3 deadline service (P7 — pinned
   as refusal until then).
5. gate3 browser registration (MUST-FIX-BEFORE-REGISTER preconditions, §3).
6. The bytecode-linking brief (spawn_link/monitor family + link propagation on
   bytecode exits — WPORT-4 Ruling 7's latent gap stays unreachable until it).
7. `erlang:spawn/4` (node-qualified) — whether a single-node cooperative
   remote-spawn seam should exist at all (brief-vs-bytes flag, see the
   `erlang:spawn/4` row).
