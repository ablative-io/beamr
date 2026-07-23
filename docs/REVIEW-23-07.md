# beamr — Repository Review

**Reviewed at:** commit `fd71c5e` (branch `main`), 2026-07-23
**Scope:** whole repository — architecture, code quality, correctness, and design-doc drift
**Method:** 11 parallel subsystem/doc reviewers (Opus), each finding adversarially verified by an independent refuter that re-read the code; the two highest-severity findings were then re-traced by hand for this report. Build, clippy, and the full test suite were run directly.

---

## Verdict

beamr is a genuinely impressive, unusually disciplined from-scratch BEAM VM (~140k lines of Rust). The concurrency core, interpreter, GC refcounting, loader hardening, and JIT deopt machinery are engineered with real care and backed by ~1,984 tests that all pass. The codebase is **not** in a shippable-for-untrusted-input state, however: there are **two confirmed critical memory-safety bugs** reachable from ordinary Gleam/Erlang workloads, a cluster of GC-safety and exception-fidelity defects, and systematic resource-exhaustion exposure on the distribution and ETS surfaces. Design docs are well above average for a project this size, with drift concentrated in the two oldest core docs and a few stale status banners.

**Health snapshot (verified directly):**
- `cargo build --workspace` — clean
- `cargo clippy --workspace --all-targets -D warnings` — clean (0 warnings)
- `cargo test --workspace` — **1,984 tests pass, 0 fail** (1,730 lib unit + integration + doc)
- Gates battery (`gates.json`): fmt / clippy / wasm32-check / tests, evidence bound to tree hashes — a sound CI discipline

---

## Confirmed findings

Severity reflects the corrected severity after adversarial verification. File:line are as of the reviewed commit.

### CRITICAL

#### C1 — GC refc-release walk misclassifies headerless cons cells as `ProcBin`, calling `Arc::from_raw` on arbitrary memory
**`crates/beamr/src/process/heap.rs:150`** (+ `gc/mod.rs:288`) — *memory-safety* — **hand-verified for this report**

`Heap::visit_allocated_boxed_objects` reconstructs each allocation's type by reading its first word as a boxed header. Cons cells are **headerless** — `write_cons` (`term/boxed/mod.rs`) writes `heap[0] = head.raw()`, so word[0] of a `[false | _]` cell is the encoded atom `false`.

The encoding collides exactly: `Term::atom(Atom::FALSE)` = `(3 << 3) | 0b001` = `0x19`, and `BoxedTag::ProcBin = 0x19`. `BoxedHeader::tag(0x19) & 0xFF → Some(ProcBin)`, `size(0x19) = 0 → object_words = 1 ≤ 2`, so the cons is walked as a live ProcBin. `release_refcounted_resources_in_young` then calls `release_proc_bin_arc(ptr)` **unconditionally** (the `is_forwarded` check only gates byte accounting, not the release), which does `read_raw_word(ptr, 2)` — an out-of-bounds read past the 2-word cons into the adjacent allocation — reinterprets that word as `*const Vec<u8>`, executes `Arc::from_raw(..)` + drop (freeing/decrementing an Arc at an attacker-influenced address), then writes `0` over the neighbouring heap word.

**Failure scenario:** any process holding a boolean list `[False, ...]` (pervasive in Gleam) in its nursery at the next minor GC triggers arbitrary-memory free + heap corruption. This is the single most serious defect in the tree. It also fires for `eexist`-class atoms at index ≡ 3 mod 32, and can collide opportunistically for boxed/list pointers whose low byte coincidentally equals a valid tag.

**Note:** one memory reviewer gave the subsystem a clean bill on unsafe-block/refcount-balance grounds — that audit is correct *for genuine ProcBins*; the bug is the spurious visitation of non-ProcBins, which that audit did not cover. The existing GC proptests never build a list with head `false`, so it is unexercised.

**Fix direction:** the refc-release walk must distinguish headerless cons cells from headered boxed objects (record an object kind at allocation time, or tag cons allocations), not infer type from word[0].

#### C2 — ETS `insert` stores un-copied process-heap terms (use-after-free)
**`crates/beamr/src/ets/set.rs:41`, `native/ets_bifs.rs:244`** — *memory-safety* — **confirmed by two reviewers + hand-verified**

`parse_insert_objects` returns bare caller-heap `Term`s (no copy), and `EtsSet`/`EtsOrderedSet`/`EtsBag` store them directly in `DashMap<EtsKey, Term>`. The GC is a **per-process generational copying collector** (`gc/mod.rs:3`) that moves and frees young-heap terms; ETS rows are not roots of the owning process. The crate *documents its own invariant* — `ets/copy.rs:4`: "ETS entries cannot point into a process heap" — and ships the `OwnedTerm`/`copy_term_to_ets` deep-copy facility, but wires it only into the `give_away`/heir path (`ets_bifs.rs:57`), **not** into `insert`.

**Failure scenario:** process A does `ets:insert(T, {k, <<boxed>>})`, keeps running (any allocation → minor GC moves/zeroes the young region), then any process does `ets:lookup`/`tab2list`/`select` and dereferences the dangling pointer — use-after-free; for `ordered_set`, even internal `BTreeMap` key comparisons walk freed memory. Tests pass only because no GC runs between insert and lookup.

**Fix direction:** call `copy_term_to_ets` on insert and `copy_term_to_heap` on lookup, storing `OwnedTerm` in the table backing maps.

### HIGH

#### H1 — `try_case_end` raises `{badmatch, V}` instead of `{try_clause, V}`
**`crates/beamr/src/interpreter/opcodes/exceptions.rs:81`** — *correctness*
A `try Expr of Pat -> ... catch ... end` whose success value matches no `of` clause reports the wrong catchable reason (`try_clause` isn't even in the atom table). `error:{try_clause,_}` handlers miss; `error:{badmatch,_}` handlers wrongly fire.

#### H2 — Body-position `bif`/`gc_bif` failure is uncatchable and mislabeled as `badarg`
**`crates/beamr/src/interpreter/opcodes/guards.rs:255`** — *correctness*
Body-position BIF failures (fail label 0) return `Err(ExecError::Badarg)`, which propagates straight out of `run_loop` via `?`, **bypassing the process's own try/catch stack**, and discards the native's real class/reason. `1 div 0` should be a catchable `{badarith,_}`; here it is an uncatchable, mislabeled crash. This is the exact bug-class the recent `func_info`/`if_end` fixes addressed — these are live siblings. Fix: raise via `exceptions::raise_exception` the way `native_call.rs` already does.

#### H3 — `jit_bs_start_match` stores a stale source binary across a GC-capable allocation
**`crates/beamr/src/jit/runtime_binary_match.rs:36`** — *memory-safety*
The helper captures the source binary `Term` before an allocation that can trigger a moving minor GC, then writes the pre-move pointer into the match context. After a GC move the match reads relocated/freed memory.

#### H4 — JIT map-update helper writes stale term pointers after a GC-capable `ensure_space`
**`crates/beamr/src/jit/runtime_map.rs:135`** — *memory-safety*
`write_map_entries` collects key/value `Term`s, then calls `ensure_space` (may collect and move), then writes the **pre-move** pointers into the freshly allocated map.

#### H5 — string BIFs hold a heap-interior `&'static` slice across a GC-triggering allocation
**`crates/beamr/src/native/stdlib_stubs/string_bifs.rs:339`** — *memory-safety* — **hand-verified**
`binary_bytes` launders a `BinaryRef::as_bytes()` into `&'static [u8]`; for inline heap binaries this points into the GC-managed process heap. `bif_split`/`bif_find`/`bif_trim`/`bif_slice` pass that slice into `context.alloc_binary`, whose `ensure_heap_space` can run a moving collection. The slice is not a root, so `string.split`/`slice`/`trim`/`contains` on a small binary near a full heap copies from stale/freed memory → silently corrupt result or crash. These functions bypass the re-root-after-reserve pattern used everywhere else in the file.

#### H6 — Unbounded frame allocation from a remote distribution header aborts the node
**`crates/beamr/src/distribution/connection.rs:1477`** — *security (DoS)*
The live read loop takes `control_len` and `payload_len` as full `u32`s from the peer, guards only `checked_add` overflow, then `let mut frame = vec![0_u8; total_len];` — infallible allocator, no max-frame cap. A peer past the handshake sends an 8-byte `0xFF..` header claiming ~8 GiB: 8 bytes of input → gigabytes of RAM (per frame), or `handle_alloc_error` **aborts the whole node**, dropping every other connection and process. The same module already has the safe pattern (`etf.rs:151` uses `try_reserve_exact → LengthTooLarge`), so this is an inconsistency, not a necessity.

### MEDIUM

**Correctness / semantic fidelity**
- **M1 — Binary integer matching of >60-bit values raises uncatchable `badarg` instead of binding a bignum.** `interpreter/opcodes/binary/matching.rs:83` — `<<X:64>>` with value ≥ 2^60 hits `try_small_int → None → Err(Badarg)` (uncatchable) though bignums exist elsewhere.
- **M2 — `lists:member/2` (and `keyfind`/`keystore`/`keydelete`) use `==` numeric coercion instead of `=:=` exact equality.** `native/stdlib_stubs/lists_bifs.rs:81` — `lists:member(1, [1.0])` returns `true` (OTP: `false`); key ops match/replace/delete the wrong tuple across int/float.
- **M3 — `maps/*` BIFs return `badarg` where OTP raises `{badmap, Map}`.** `native/stdlib_stubs/maps_bifs.rs:537` — no `badmap` atom exists in the crate; error-handling that distinguishes `badmap` silently mis-branches.
- **M4 — Distribution big-integer decoder rejects `u64`-range positives its own encoder emits.** `distribution/etf.rs:652` — `i64::try_from` fails for `SMALL_BIG_EXT` magnitudes in `[2^63, 2^64-1]` that `encode_big_bytes` produces; breaks round-trip and replay-file restore. The runtime decoder handles this correctly via `u128` + bigint fallback.
- **M5 — Instruction validator never resets `current_frame_size` across function boundaries / tail calls.** `loader/validate.rs:211` — `CallLast`/`CallExtLast`/`ApplyLast`/`FuncInfo` don't clear the frame bound, so a leaf function following a tail-call-terminated one passes Y-register validation against a frame it never allocated. A defense-in-depth soundness gap against malformed `.beam`.
- **M6 — `ExecError` `Display` cannot resolve user atoms.** `error.rs:203` — the `Display` impls for `Undef`/`Badfun`/`Badarity`/`GuardBifUnavailable` build a fresh `AtomTable::with_common_atoms()` (only ~79 built-ins), so `format!("{err}")` on a real MFA prints `undefined function #<unknown atom>:#<unknown atom>/1`. Actively misleading; the real data only appears via `format_with_atoms`.

**Resource exhaustion / DoS**
- **M7 — Unbounded atom interning (three vectors, one root cause).** `atom/table.rs:228` `intern` `Box::leak`s each name with no cap and `limit()` returns `u32::MAX`. Reachable via (a) distribution control frames interning fresh remote atoms unboundedly (`control_link.rs:267`, `safe=false`), (b) `binary_to_atom`/`list_to_atom` in a loop (`native/gate3_bifs/type_conversion.rs:51`). Permanent leak + eventual u32 index wrap; `system_info` reports a finite `atom_limit` that nothing enforces. Real BEAM caps the table and halts loudly.
- **M8 — Unbounded native recursion in term comparison and hashing.** `term/compare/mod.rs:432` — deeply nested terms overflow the native stack (GC itself uses an explicit work queue and is safe; compare/hash do not).
- **M9 — ETS match-spec allocates a bindings vector sized by the raw `$N` variable index.** `ets/match_spec.rs:207` — `ets:select(T, [{{'$100000000'}, [], ['$100000000']}])` allocates ~1.6 GB (or ~68 GB at `$4294967295`) per clause eval; any process that can call `select` can abort the VM.

**Concurrency / messaging**
- **M10 — io_uring / thread-pool ring shutdown hangs on an in-flight blocking op.** `io/uring.rs:211`, `io/thread_pool.rs:226` — the worker loop only exits when `in_flight` is empty and there is no cancellation SQE; a pending `Accept`/`Read` whose CQE never arrives makes `Drop`→`shutdown()`→`join()` block forever.
- **M11 — Per-sender message FIFO inversion.** `scheduler/execution/core.rs:442` — `store_runnable_process` appends `Executing`-deferred pending messages to the back of the scan list without first draining the arrival queue, so on a compute-only slice an older arrival message can order after a newer pending one — a BEAM ordering-guarantee violation. The debugger snapshot path drains first, showing the authors know the correct order.
- **M12 — Local send to a `Present` receiver silently drops the message on `HeapFull`.** `scheduler/supervision_integration.rs:1004` — the code holds `&mut Process` under the slot lock and the parallel I/O path proves `ensure_space` is usable here, yet this path drops instead of collecting — silent, reachable message loss. (Related: closures with captured variables sent to an `Executing` receiver are dropped by the ETF encode path, making delivery race-dependent.)

**Performance**
- **M13 — `tcp_connect` does blocking DNS on the scheduler thread.** `native/tcp_bifs.rs:551` — `to_socket_addrs()` (sync `getaddrinfo`) runs inline before the async submit; a slow/unreachable hostname stalls every process on that scheduler for the resolver timeout — the exact starvation the completion-ring design avoids everywhere else.

**Documentation drift** (docs say X, code does Y — verified against both)
- **D1 — `beamr-vm-design.md:421` scope table marks Distribution and JIT as "Skip / not needed"** — both are large shipped subsystems (13.6k and 15k lines).
- **D2 — `beamr-vm-design.md:23` claims the Gleam compiler "produces `.beam` bytecode" directly** — it goes through `erlc`; the repo's own `terminology.md`/`README` say so. The doc's "Crate Structure"/`Vm::new()` API also does not exist in the code.
- **D3 — `AOT-NORTH-STAR.md:136` lists two "known correctness bugs" (cross-process send, recv_marker) as current** — both fixed per CHANGELOG (0.7.0/0.16.0). Lowering figure "~51 of 66" is also stale (code classifies ~60 of 75).
- **D4 — `WASM-RUNTIME-PORT-DESIGN.md:14` headline premise ("native processes are the gap")** — the port has landed (`wasm_native.rs`, beamr-wasm actor surface); §10 is synced but §0–§9 still read as unbuilt future plan.
- **D5 — `DISTRIBUTION-HANDSHAKE-DESIGN.md:153` states the wrong simultaneous-connect survivor** — prose says "lower-named node's outbound survives" (3×), but the doc's own OTP rule table and the landed code make the **higher-named** node's outbound win. Misleads anyone reasoning about which half-link is authoritative.
- **D6 — `CONN-EVENTS-HOOK-SPEC.md:401` arm table / INV-EXACTLY-ONCE omits the live-incumbent peer-bounce path the code implements** — `connection.rs:1245-1328` emits Down(old)→Up(new) on a live-but-stale peer restart; the spec says such displacement is invisible. `connection_events.rs` documents the extension; the spec was never updated.

---

## Latent / dormant (real defect, currently unreachable)

**L1 — Typed-int deopt leaves untagged payloads in X registers; interpreter re-runs from entry reading them as tagged terms.** `jit/compiler/ir_typed.rs:251`
The mechanism is real and fully traced: `initialize_entry_values` untags Int X-registers in place in the process register file, the typed arithmetic/frame-guard deopt edges don't re-tag, and on deopt the interpreter re-executes from entry reading corrupted registers. **However**, it cannot execute in the current tree: typed native code is produced only by `AotCompiler::compile_module`, whose output either never reaches live dispatch (AOT cache loads at generation 0 while dispatch looks up generation ≥1 — "inert today", `aot.rs:268-273`) or is recompiled untyped. The demand-JIT worker uses the untyped compiler. This will become a live critical the moment typed-AOT dispatch is wired up — it deserves a tracking issue and a differential test gating that work, but it is not a live bug today.

---

## Low / hygiene

- Empty 0-byte `aion.db` and `.commit-msg.tmp` committed at repo root, not gitignored.
- **Gate battery builds default features only** (`gates.json`) — `telemetry` (1,384 LOC), `encode`, and native `json`-gated code are never compiled under `-D warnings` nor tested; a regression behind those cfgs passes GREEN. Consider an `--all-features` leg.
- `meridian_ffi.rs` — app-specific POC NIFs (including a `run_cmd/1` that shells out via `sh -c`) live in the reusable core crate under the default `fs` feature and are registered by `beamr-cli`. Not silently exposed to library embedders (registration is explicit), but it couples the VM to sibling projects and ships POC scaffolding as production surface. Move to the CLI or an example, or feature-gate separately.
- `DOCUMENTATION.md:86` pins the dependency at `0.15`; crate is `0.16.0`.
- `READINESS-CONTRACT-SPEC.md` still headed "DRAFT v0 … Not for implementation" though the feature ships by default and its pinning suite exists.
- Module-wide `#![allow(dead_code)]` in `interpreter/pattern.rs:8` silences dead-code detection across a core file.

---

## Strengths (verified, worth preserving)

- **Concurrency core is exemplary.** The park/wake protocol (`execution/core.rs`) enumerates and defends all three delivery-vs-park interleavings and closes the store→register kill-gap; suspension keys completions by monotonic call-id with newest-wins + liveness recheck; lock ordering is consistent (no deadlock found); work-stealing uses deliberate FIFO owner-pop with a single-item no-steal rule and low-priority fairness windowing; the timer wheel is correctly O(1) including far-future wraparound. Link/monitor/exit semantics match BEAM.
- **Interpreter fidelity is high** and well-tested: catch/try wrapping shapes, receive markers, stacktrace-derivation-at-read, closure/apply across module reloads, GC-reserve-before-write. The recent `func_info`/`if_end` fixes are correct and documented inline.
- **GC refcounting is genuinely correct for real ProcBins/FdResources** across all four transitions (minor promote, major compact, unreachable collection, failed-minor-then-major) — refcounts balance. Root enumeration is complete over all stack frames. Constant-pool literals are correctly excluded from collection.
- **Loader is systematically hardened** against malformed input: checked offset arithmetic, a shared `DecodeBudget` (node/byte/atom/depth ceilings), a bounded inflater defeating zip bombs, and `ensure_count` before nearly every `Vec::with_capacity` (the compact decoder, H7/M-cluster, is the one gap). The unsafe io_uring/thread-pool FFI keeps every buffer alive to its CQE with accurate SAFETY notes.
- **Docs are well above average** for a 140k-line VM. `terminology.md` is precise and self-correcting; the readiness/embedder-composition specs (`READINESS-REGISTRATION-API.md`, `EMBEDDER-COMPOSITION-SPEC.md`) match the code closely; README's subsystem inventory and "1,500+ tests" claim are accurate (conservative — ~1,984).
- **JIT is disciplined for a from-scratch effort:** the `jit::coverage` classification tier is exhaustive-no-wildcard (no misclassification found), the deopt-after-side-effect guard is a proper monotone forward dataflow over a real CFG, heap-allocating lowerings re-read operands after alloc, and typed registers never leak an untagged payload into a GC-rooted Y slot.

---

## Recommended priority order

1. **C1, C2** — the two critical memory-safety bugs. Both are reachable from ordinary Gleam code and both have clear fixes (record object kind for the refc walk; wire `copy_term_to_ets`/`copy_term_to_heap` into ETS insert/lookup).
2. **H3, H4, H5** — the GC-move-across-unrooted-Term class in JIT runtime helpers and string BIFs; audit the whole crate for `Term`/slice held across `ensure_space`/`alloc_*`.
3. **H1, H2, M1** — exception fidelity: the "returns `Err(ExecError)` where it should `raise_exception`" bug-class is uncatchable-and-mislabeled; sweep for remaining siblings.
4. **H6, M7, M9** — untrusted-input DoS on distribution/ETS before any multi-node or untrusted-bytecode deployment.
5. Doc drift (D1–D6): add a "superseded — see README Architecture" banner to `beamr-vm-design.md`, correct the handshake survivor prose, and sync the conn-events arm table.
