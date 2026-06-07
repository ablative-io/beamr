# beamr Brief Tracker

Current version: **0.3.13** | Tests: **682** | Published: 2026-06-07

## Phase 0 -- Foundation (COMPLETE)

All briefs landed on main. 45 briefs, v0.1.0 through v0.3.12.

| Brief | Title | Status |
|-------|-------|--------|
| B-001 | Term representation and tagging | Landed |
| B-002 | Atom table and interning | Landed |
| B-003 | Process struct and X/Y registers | Landed |
| B-004 | BEAM loader -- header and atom chunk | Landed |
| B-005 | BEAM loader -- code chunk decoding | Landed |
| B-006 | BEAM loader -- import/export tables | Landed |
| B-007 | Interpreter loop and basic dispatch | Landed |
| B-008 | Arithmetic and comparison BIFs | Landed |
| B-009 | Tuple operations | Landed |
| B-010 | List operations (cons, hd, tl) | Landed |
| B-011 | Pattern matching (select_val, test_arity) | Landed |
| B-012 | Function calls (call, call_only, return) | Landed |
| B-013 | External calls and import resolution | Landed |
| B-014 | Allocate/deallocate stack frames | Landed |
| B-015 | Exception handling (try/catch) | Landed |
| B-016 | Process spawning and PID terms | Landed |
| B-017 | Scheduler -- multi-threaded run loop | Landed |
| B-018 | Message passing (send/receive) | Landed |
| B-019 | Selective receive with save pointer | Landed |
| B-020 | Links and exit signals | Landed |
| B-021 | Monitors and DOWN messages | Landed |
| B-022 | Trap exit and EXIT messages | Landed |
| B-023a | Binary term representation | Landed |
| B-023b | Binary construction (bs_init, bs_put) | Landed |
| B-023c | Binary matching (bs_start_match, bs_get) | Landed |
| B-024 | Map operations | Landed |
| B-025 | Closures and make_fun2 | Landed |
| B-026 | Float terms and boxing | Landed |
| B-027 | Big integer support | Landed |
| B-028 | Reference terms | Landed |
| B-029 | Generational garbage collection | Landed |
| B-030 | Timer wheel and receive timeouts | Landed |
| B-031 | Process registration (register/whereis) | Landed |
| B-032 | Work stealing scheduler | Landed |
| B-033 | Gleam stdlib stubs | Landed |
| B-034 | Gleam FFI (gleam_erlang_ffi) | Landed |
| B-035 | Selector FFI | Landed |
| B-036 | OTP stubs | Landed |
| B-037 | Meridian FFI (NIF bridge) | Landed |
| B-038 | apply/apply_last opcodes | Landed |
| B-039 | beamr-cli runner | Landed |
| B-040 | E2E Gleam workflow proof | Landed |
| B-040a | Gate3 BIFs | Landed |
| B-040b | Code management BIFs | Landed |
| B-041 | Dual module versions with purge | Landed |
| B-042 | Closure version binding | Landed |
| B-043 | Dynamic import resolution | Landed |
| B-044 | Process module version pinning | Landed |
| B-045 | Hot code loading lifecycle | Landed |

## Phase 0.5 -- Hardening (DISPATCHING)

Bug fixes and hardening for the 5 open GitHub issues plus namespace isolation.

| Brief | Title | Status | Issue |
|-------|-------|--------|-------|
| B-046 | Capability-gated process access | Written | #9 |
| B-047 | Deterministic replay infrastructure | Written | #10 |
| B-048 | Loader validation boundary | Written | #11 |
| B-049 | Box::leak memory leak fix | Written | #12 |
| B-050 | Atom ordering by name | Written | #13 |
| B-051 | BIF coverage -- demand-driven expansion | Written | -- |
| B-052 | Scheduler hardening and edge cases | Written | -- |
| B-053 | Namespace infrastructure | **Landed** | -- |
| B-054 | Namespace-aware interpreter | **Landed** | -- |

### Bug fixes landed without briefs (v0.3.2--v0.3.13)

| Version | Fix |
|---------|-----|
| 0.3.2 | init_yregs opcode, executable_line no-op |
| 0.3.3 | JSON decode/encode BIFs (feature-gated) |
| 0.3.4 | Badarg context preservation |
| 0.3.5 | X register widening (256 to 1024, u8 to u16) |
| 0.3.6 | Register::X enum u8 to u16 |
| 0.3.7 | try_case writes to x(0-2) per BEAM spec |
| 0.3.8 | JSON null returns atom 'null' not NIL |
| 0.3.9 | JSON object binary keys (OTP 27 compat) |
| 0.3.10 | take_exit_exception API for error diagnostics |
| 0.3.11 | classify_dynamic returns binaries (Gleam compat) |
| 0.3.12 | call_fun2 full closure dispatch |
| 0.3.13 | Namespace isolation, exit signal race fix |

## Phase 1 -- Critical Opcode/BIF Gaps (NOT STARTED)

These block real Erlang code. ~20 briefs estimated.

| Area | Items | Priority |
|------|-------|----------|
| **get_list opcode (65)** | List destructuring -- almost all Erlang pattern matching | CRITICAL |
| **trim opcode (136)** | Stack trimming in tail calls | CRITICAL |
| **swap opcode (169)** | Register swap, common in OTP 27+ | CRITICAL |
| **catch/catch_end (62/63)** | Traditional exception handling | CRITICAL |
| **build_stacktrace (160)** | Stack trace construction | HIGH |
| **raw_raise (161)** | Re-raise exceptions | HIGH |
| **is_tagged_tuple (159)** | Record pattern matching | HIGH |
| **Float ops (96-102)** | All 7 float arithmetic instructions | HIGH |
| **update_record (181)** | Record update syntax | HIGH |
| **erlang:throw/1** | Exception type missing | CRITICAL |
| **Comparison ops ==, >, =<** | Numeric equality vs exact | HIGH |
| **Process dictionary** | put/get/erase -- new subsystem | HIGH |
| **Binary ops (13 remaining)** | bs_skip, bs_get_float, UTF, bs_match | HIGH |
| **Tail-call BIF return** | call_ext_only with BIF doesn't return | MEDIUM |
| **recv_marker opcodes (173-176)** | OTP 24+ selective receive optimization | MEDIUM |

## Phase 2 -- Platform (NOT STARTED)

Makes OTP libraries work. ~30 briefs estimated.

| Area | Items | Est. Briefs |
|------|-------|-------------|
| **ETS** | Tables, match specs, concurrent access | 5-8 |
| **Port/IO on io_uring** | Modern async I/O, file, sockets | 10-15 |
| **Refc binaries** | Reference-counted + sub-binary + binary heap | 3-5 |
| **Dirty schedulers** | Long-running NIF support | 2-3 |
| **Process priorities** | low/normal/high/max scheduling | 1-2 |
| **spawn_monitor** | Common spawn pattern | 1 |
| **process_info/1,2** | Process introspection | 1-2 |
| **system_info/1** | VM introspection | 1 |
| **group_leader BIFs** | Read/write group leader | 1 |

## Phase 3 -- Full Replacement (NOT STARTED)

Makes Elixir work, enables clustering. ~40 briefs estimated.

| Area | Items | Est. Briefs |
|------|-------|-------------|
| **Distribution protocol** | Node-to-node, remote PIDs, net_kernel | 20-30 |
| **External term format** | binary_to_term/term_to_binary | 3-5 |
| **Full BIF coverage** | Remaining ~230 erlang BIFs | 10-20 |

## Phase 4 -- Beyond BEAM (NOT STARTED)

Innovations the original BEAM can't do. ~30 briefs estimated.

| Area | Items | Est. Briefs |
|------|-------|-------------|
| **JIT compilation** | Cranelift-based adaptive compilation | 8-10 |
| **Deterministic replay** | Record/replay debugging | 5-8 |
| **WASM target** | BEAM processes in browser/edge | 5-8 |
| **Capability security** | Per-process capability gates | 3-5 |
| **Structured observability** | OpenTelemetry-native tracing | 3-5 |

## Open GitHub Issues

| Issue | Title | Severity | Phase |
|-------|-------|----------|-------|
| #9 | meridian_ffi capabilities | MEDIUM | Phase 0.5 (B-046) |
| #10 | Determinism/replay | LOW | Phase 0.5 (B-047) |
| #11 | Loader validation boundary | MEDIUM | Phase 0.5 (B-048) |
| #12 | Box::leak memory leak | HIGH | Phase 0.5 (B-049) |
| #13 | Atom ordering by name | LOW | Phase 0.5 (B-050) |

## Known Bugs (not yet filed)

- Tail-call BIF return: `call_ext_only` with Native BIF target returns `Continue` instead of returning from function. Masked by flat code layout falling through to next function.
- Nested closure dispatch: `json.parse` + `decode.string` decoder pipeline crashes with nested closures in `gleam@dynamic@decode`. Not blocking (Aion decodes JSON on Rust side).
- trap_exit + non-Kill exit signal to process being executed: can't check trap_exit flag when body is taken, tombstones anyway. Needs signal queue for full correctness.
