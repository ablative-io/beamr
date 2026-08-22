# Changelog

## Advisory — JIT-compiled code silently dropped every message it sent to another process

**Affects 40 of the 58 published versions: every version from `0.4.0` through
`0.18.1`, with no holes in the published sequence. FIXED IN `0.18.2`** — see
that entry's "Fixed" section below.

**`0.18.2` reaches no existing consumer on its own.** Each `0.x` minor is a
semver major, so `^0.16` and `^0.17` requirements **cannot** resolve `0.18.2`
and are not carried by it — moving across a minor is a manifest edit, not a
lock refresh. **The `0.16.x` line is unpatched, and backporting this fix to it
was considered and ruled against** — the runtime seam the fix reaches through
does not exist at that base, so a `0.16.x` fix is a redesign rather than a port.
A version is patched only when it appears in this file with its own entry
saying so; this file is the list, and it is appended to as releases are cut.

**What happens.** With the JIT enabled, once a function has been compiled, a
`!` executed from that compiled code delivers **only when the destination is
the sending process itself**. Every other destination — another local process,
a remote process, a non-pid — is **silently discarded**, and the expression
still evaluates to the message term, which is exactly what a successful `!`
produces. There is no error, no crash, no log line, and no deopt: the send
returns the success value and the message never arrives. The same program run
without the JIT delivers correctly, so a suite that passes under interpretation
proves nothing about a process that has run hot enough to tier up.

**A second defect in the same helper.** The one destination that *was*
implemented — the self-send — pushed straight to the mailbox and skipped the
sender's logical-clock tick and the replay driver's delivery check, both of
which the interpreter performs. Compiled self-sends therefore diverge from
interpreted ones in recorded/replayed executions, silently.

**Exposure.** The defect is in the JIT runtime helper, so it is reached only
after a function tiers up — which means it lands on exactly the hot paths a
long-running system depends on, and only after it has been running a while.

- **`0.4.0` and `0.4.1`** — there is **no `jit` cargo feature**: the module is
  declared with no cfg gate and the cranelift dependencies are non-optional.
  The defective code compiles **unconditionally and cannot be disabled by
  feature selection** in these two versions.
- **`0.4.2` through `0.18.1` (38 versions)** — the `jit` feature exists, gates
  the module, and is **in `default`** in every one of them. Default builds
  carry the defect. A build with `default-features = false` and without `jit`
  does not compile the module at all. **There is no published version where
  `jit` exists but is not on by default.**
- **`0.1.0` through `0.3.15` (18 versions) are unaffected** — they ship no
  `src/jit/` directory and no `jit` feature.

**How the range was established.** From the **packaged bytes of every published
`.crate`**, downloaded from static.crates.io, with the version list taken from
the crates.io registry API — **not from git tags**. All 58 downloads, all 58
extractions and all 58 measurements succeeded; there are no
failed-to-measure rows. Exactly **one** body variant of the helper exists
across all 40 carriers (single body hash `e350f274…`), independently confirmed
by byte-level diff of the first, a middle and the last packaged copy. The two
yanked versions (`0.13.1`, `0.13.2`) are inside the affected range and are
counted in the 40.

🔴 **CORRECTION, recorded because the error ran the wrong way.** This range was
first stated internally as beginning at `0.10.0`, read from the first line of
`git tag --contains <the defect commit>`. **That output is sorted
lexically, not by version — `v0.10.0` merely sorts before `v0.4.0`.** The
measured first-affected version is `0.4.0`, confirmed both by the packaged
bytes and by `git merge-base --is-ancestor`. A first line is not a minimum,
and an affected range is a measured artifact, never a reasoned one. Had the
undercount shipped, every reader on `0.4.x`–`0.9.x` would have read the
advisory as "not me".

**Not attributed to any incident.** This advisory rests on the two defects
above **on their own merits** — silent message loss and silent replay
divergence, evidenced by the packaged-bytes census. It is deliberately not
tied to any production failure: one such attribution was investigated during
this work and **retracted** when the mechanism was shown not to execute on the
path in question.

## Advisory — silent memory-safety defects in every version below 0.16.3

**If you are on any version below `0.16.3`, upgrade.** Three classes of
silent memory-safety defect were fixed across `0.16.2` and `0.16.3`. None of
them produce an error or a crash — the failure mode in every case is
corrupted or freed data read as valid, so a suite that passes proves nothing
about exposure.

🔴 **CORRECTION (2026-08-07): THIS ADVISORY PREVIOUSLY SAID "IF YOU ARE ON
`0.16.0` OR `0.16.1`". THAT WAS AN UNDERCOUNT, AND IN A SAFETY DISCLOSURE THE
UNDERCOUNT IS THE DIRECTION THAT HURTS** — a reader on `0.15.4` read it as
"not me". Measured: `crates/beamr/src/native/stdlib_stubs/string_bifs.rs`,
which carries the `as_bytes` launder (`binary_bytes` returning
`&'static [u8]`) and all five affected consumers (`bif_trim`, `bif_split`,
`bif_find`, `bif_pad`, `bif_slice`), is **byte-identical — blob
`d4054622d770886a7d91e1748cfd078d172206e3` — across all 29 tags from `v0.4.4`
through `v0.15.2`, and at `67f89c4`** (the `0.16.2` commit). The launder's
signature is present from `v0.2.0`. So the class fixed in `0.16.3` spans
essentially the crate's whole published history, not one minor line.

**The two classes fixed in `0.16.2` have NOT had their introduction point
measured.** They are stated here as unmeasured rather than assumed narrow.
Until someone measures them, treat any version below `0.16.3` as affected.

The original wording was not wrong about `0.16.0`/`0.16.1` — it was wrong
about everything it left out. A version enumeration in a disclosure is a
claim about the versions it *omits*, and nothing in the sentence marked which
of those two jobs it had done.

- **Fixed in `0.16.2`** — two classes: the GC refcount-release walk
  inferred a type from `word[0]` and could call `Arc::from_raw` on a
  heap-cons payload; and ETS stored borrowed caller-heap terms that
  outlived the heap they pointed into.
- **Fixed in `0.16.3`** — the `as_bytes` borrow-across-alloc class
  (nine BIF crossings, `binary_to_term`, and `jit_bs_get_binary`),
  detailed below.

**Pinning.** `0.16.3` is available as the `v0.16.3` git tag and as a
published version. `0.16.2` has no tag and is pinned by commit
`67f89c4`. Anything cut from a base that does not contain `67f89c4`
carries the two 0.16.2 classes regardless of its version number — check
with `git merge-base --is-ancestor 67f89c4 <base>`.

**The "Known remaining JIT sites" class is FIXED in `0.18.1`** — see that
entry's "Fixed" section. `0.16.3`, `0.17.0` and `0.18.0` all carry it,
reachable under the `jit` feature, which is **on by default** in those
versions. The disclosure as first written is in the `0.16.3` entry; its
fullest statement is in the `0.17.0` entry. This sentence enumerates
versions, and **the enumeration is a claim about the versions it omits**:
every version below `0.18.1` carries the class, and the fix lane's own
sweep at the fix commit shows zero remaining REAL-verdict sites under
`crates/beamr/src/jit/`
(`docs/design/beamr/briefs/evidence/review-23-07/rf-006/sweep/verdicts.json`).

**`0.18.1` is still not a clean bill of health.** A second rooting class —
accumulator rooting in native BIFs, **no JIT required**: a term accumulated
into a `Vec` or threaded `tail` goes stale when a later allocation in the
same loop collects, and the terminal `alloc_list`/`alloc_tuple` roots a
pointer that is already stale — is **open, sized and unfixed in every
released version including `0.18.1`**: 17 verified crossings, one shape,
enumerated by name in
`docs/design/beamr/briefs/evidence/accumulator-rooting/dispositions.json`,
with a pre-registered landing gate at
`docs/design/beamr/briefs/AR-1-LANDING-GATE.md`. Two of the seventeen have
demonstrated red-at-parent probes; the class is real, not theoretical. Four
further sites cleared on unaudited word-count arithmetic are recorded
`UNRULED-PRERESERVE` (gate row 3) — unruled, not safe. The audit ships in
this tree **before** any fix exists, deliberately, so the accounting cannot
be derived from the post-fix state.

🔴 **`jit` CANNOT BE DISABLED IN ANY BUILD THAT RETAINS `threads`.** The
manifest declares `jit = ["std", "threads", …]`, but parts of the scheduler
are gated on `threads` while referencing `crate::jit`, so a build with
`threads` and without `jit` does not compile. Disabling this class therefore
requires giving up `threads` as well. **If you run threaded beamr you carry
this surface, and turning the feature off is not a mitigation open to you.**
**That is a defect under repair, not an intended property.** The exact
failing command is in the `0.17.0` entry; the coupling predates `0.17.0` and
is present at `v0.16.3` too.

*(A paragraph naming RF-006 as this class's owner stood here from `0.17.0`
until `0.18.1`. Its own text required that when the fix landed, the release
carrying it would say so under "Fixed" and the paragraph be removed in the
same commit. That is this commit — see the `0.18.1` entry.)*

## 0.20.0 — 2026-08-22

### Changed
- **BREAKING: heap-backed binary and bignum accessors are now tied to a heap
  borrow.** `Binary::as_bytes`, `BinaryRef::as_bytes`, `ProcBin::as_bytes`,
  `SubBinary::as_bytes` and `BigInt::limbs` each take a `HeapBorrow<'heap>` and
  return `&'heap [u8]` / `&'heap [u64]`.

  **What this fixes.** Those accessors returned a slice whose lifetime was not
  tied to anything — in practice a `&'static [u8]` pointing into a process
  heap that could be collected, reallocated or moved while the slice was still
  live. Nothing in the type system objected. The dangle was demonstrated
  against the unfixed tree first, and the same construction is now a **compile
  error**: the proof is a `compile_fail` doctest paired with a runnable
  control, so a proof that stops compiling for some unrelated reason cannot
  quietly pass as evidence.

  Fifteen sites were found and retired; no `'static` return over heap memory
  remains in the tree. The signature change ripples through 83 source files.

  **Migration.** Pass a borrow at the call site — `binary.as_bytes(heap.borrow_terms())`
  where a `&mut Heap` is in hand, or `binary.as_bytes(context.borrow_terms())`
  inside a native BIF context. Where the bytes must outlive the borrow, own
  them (`.to_vec()`). Note that `SharedBinary::as_bytes` and
  `BigIntValue::limbs` are **unchanged** — those types own their bytes rather
  than pointing into a heap, so callers of those two need no edit.

- **BREAKING: `beamr-cli` 0.7.0 and `beamr-wasm` 0.10.0 move with this cut.**
  Both pin `beamr` by version, and a `0.19` requirement cannot resolve
  `0.20.0` — each `0.x` minor is a semver major, so the three pins move
  together or not at all. `gleam-types` is untouched at 0.4.4.

### Removed
- **BREAKING: `EtsOrderedSet::new` is deleted** (release obligation OB-001).
  The constructor built a private atom table holding only the common atoms, so
  any atom interned by the VM beyond those failed to resolve and key
  comparison silently degraded to raw intern-index order. It had carried a
  `#[deprecated]` note saying exactly that, and was retained only so that
  out-of-tree callers of the publicly re-exported type would not break.

  **Use `EtsOrderedSet::with_atom_table` with the VM atom table**, which is
  what the registry already does and the one path that orders atom keys
  correctly. Deleting an inherent constructor is a breaking change, so it was
  parked for the next breaking window; this is that window.

### Added
- **`beamr-wasm` now ships a README and declares it** (release obligation
  OB-005). The crate published without `readme` metadata, so its registry page
  had no body. The README is derived from the crate's own build script,
  manifest and host seams rather than written from memory.

### Known cost — filed, unpriced
- **Tying the accessors to a borrow introduced heap→Vec→heap copies on three
  hot paths, and their magnitude is unmeasured** (beamr#35). A borrow cannot
  span `alloc_words(&mut heap)`, so a copy that previously wrote a borrowed
  slice straight into the destination now materialises an owned `Vec` first.

  Twelve net-new allocations across three families: **mailbox send** (3 — per
  boxed term on every inter-process send), **ETS** (5 — of which four are on
  *read-back* via `copy_term_to_heap`, and only one on insert), and **binary
  matching** (4 — `bs_get_binary` and `bs_get_tail`, per opcode rather than
  per message).

  No number is attached to this because no existing benchmark can see it: the
  two bench targets in the tree are both feature-gated on `jit` + `threads`
  and drive integer `fib`, so they touch none of these paths, and a green from
  either would be zero evidence. A benchmark built for these paths is
  specified and queued. `bs_get_integer`, `bs_get_float` and `bs_match_string`
  took the same signature change and allocate nothing, which gives that
  benchmark its null arm.

## 0.19.4 — 2026-08-20

### Added
- **`JitProfiler::recorded_call_count` and `JitProfiler::profile_entry_count`
  are now reachable from a release build.** Both already existed, but sat
  behind `#[cfg(any(test, feature = "test-support"))]` under the label
  "Test-support probe", so no shipped binary could call either.

  **The gate was not a boundary anyone drew.** `is_compiled`,
  `is_unsupported`, `current_threshold` and `compile_outcome_counters` are all
  already unconditionally public — and `recorded_call_count` reads the *same
  map* `is_compiled` reads, while `compile_outcome_counters` reports its
  *aggregate*. There is no policy under which those land on different sides of
  a cfg. The split was an accident of when each accessor happened to be
  written, preserved by the label sitting above it.

  The prompt was an embedder diagnosing a JIT tier-up defect: the open
  question was *which* function crosses the compile threshold — the compiled
  caller, or the awaiting callee — and that is per-MFA call-count data. It
  could not be answered from any shipped binary. The alternative, an embedder
  enabling `beamr/test-support` in its production dependencies, was rightly
  refused: that feature also gates test hooks across the scheduler,
  distribution, spawning and suspension, and pulling those into a shipped
  binary to buy one counter is the worse trade.

  Both are plain reads — one map lookup plus an atomic load, and a `len()` —
  so they cost a release build nothing unless called.

  **`profile_epoch` deliberately stays gated**, and the reason is now written
  at the accessor rather than left as residue: production already receives the
  epoch inside `RecordResult::CompileNow`, read under the deciding entry
  guard, and the accessor exists only so tests can stage stale completions.
  Ungating it would widen the API for a caller that should not exist.

## 0.19.3 — 2026-08-19

### Added
- **`Scheduler::set_jit_enabled` / `Scheduler::jit_enabled` — a runtime
  off-switch for the JIT** (#26). Until now the JIT could not be turned off in
  a running system. `SchedulerConfig::jit_threshold` is a tuning knob and not a
  substitute: it is fixed at construction, and it governs only whether code is
  *compiled*, never whether already-compiled code is *entered* — so once a
  function is cached it offers nothing, which is precisely the state an
  operator is in by the time a JIT fault has been diagnosed. (A very large
  threshold does defer compilation past any realistic workload and is a usable
  workaround before a scheduler is built; it is not a control.) Otherwise the
  only disable was the compile-time `jit` cargo feature, which is on by
  default, so an embedder meeting a JIT defect had no lever short of rebuilding
  beamr. The switch is **operator-set only** — nothing inside beamr ever writes
  it, because a runtime that disabled its own JIT on a detected fault would be
  a silent fallback rather than a control. Disabling withholds both the JIT
  cache and the profiling handle from the call edges (the engine's established
  disable mechanism), which means not only that nothing new compiles but that
  **already-compiled code is no longer entered** — the cache survives
  untouched and goes live again if the switch is turned back on. It takes
  effect at the next slice boundary; disable immediately after construction to
  guarantee nothing is ever compiled. Default behaviour is unchanged: on.
  `jit_threshold`'s saturation semantics are now documented where the field is
  declared.

### Fixed
- **A process that suspended inside JIT-compiled code kept running, and its
  host call was replayed.** When compiled code called out to interpreted code
  and that nested run suspended — a `receive`, or a host await from an embedder
  — the helper restored the *caller's* code position over the position the
  nested run had just parked at, then reported the suspension as a deopt. A
  deopt means "restart this function interpreted from its entry", so the
  compiled function's prefix ran again while the process was already parked
  with a live suspension: the external call in that prefix was re-executed,
  producing a duplicate host request, and the second park superseded the first
  so the embedder's answer to the call id it had been handed was refused. Where
  the native was re-entrant instead, the answer published successfully and the
  process exited carrying the *replayed* call's value, so the awaited result
  went nowhere. Both faces reproduced; the mechanism is pinned to the compiled
  *caller* by an arm in which the callee still compiles and the caller does
  not.

  **The same replay happened on an ordinary yield, with no suspension
  involved.** A process whose nested interpreted run simply used up its time
  slice mid-body had that resume position overwritten too, and re-entered the
  compiled function from its entry on its next slice — so any work the compiled
  prefix had already done was done again. This needs no `receive` and no
  embedder: a callee long enough to exhaust a slice is sufficient, which makes
  it the face most likely to be met by ordinary hot code. It was found by a
  probe that asserts an observable effect performed before the yield happens
  exactly once, and it is fixed.

  The contract is now explicit: **once the nested run has begun, no outcome
  that is a transfer rather than a value may have the caller's position
  restored over it.** A scheduler-level transfer — a suspension, a dirty call,
  a yield — keeps the resume position the nested run established and is handed
  to the run loop as the outcome it already is, exactly what the interpreted
  path does with the identical outcome. Every deopt taken *before* the nested
  run starts is unchanged and remains correct: nothing has been committed at
  that point and restart-from-entry is the right answer.

  This affected **both** nested-run helpers — the external-call path and the
  closure-call path (`CallFun`/`CallFun2`), which carried the same defect
  independently — and both are fixed on all three outcomes. Reported against
  aion as aion#85.
- **`==` returned `false` for arithmetically-equal bignum/float pairs**
  (#15, fixed by PR #20 — Matthew Bright). `numeric_eq` lacked the
  `BigInt↔Float` match arms and fell through to exact equality, so a bignum
  and a float of equal value compared unequal under `==` while comparing
  equal under `=<`/`>=` — an internal contradiction with the ordering path.
  The fix adds the missing arms and unifies `==` and ordering on one shared
  numeric conversion. Note: bignum↔float comparison converts through `f64`
  and rounds, matching the pre-existing ordering behavior; the
  exact-comparison gap beyond 2^53 (OTP compares losslessly) is tracked
  as #25.
- **An exception raised inside a closure called from JIT-compiled code could
  bypass the compiled frame and leak one `CallFun` nesting** (#27).
  `call_interpreted_closure` ran the nested interpreter without raising the
  nested handler floor, so an outer `catch` could unwind straight past the
  native frame that was still on the stack. `jit_call_interpreted` had guarded
  the identical region since it was written; the closure path had not. There
  are exactly two sites in the engine that begin a nested interpreted run from
  inside compiled code, and both now guard it. The ordering is load-bearing and
  not merely the presence of the calls: the floor is restored immediately after
  the nested run and *before* the transfer match, because the transfer and
  yield arms return early — a restore placed after them would be skipped on
  every transfer, leaving the floor elevated, which is the same defect in
  mirror image. Restoring on a transfer is correct: once the helper returns
  there is no Rust nesting left to protect, so the transferred-out code resumes
  as ordinary interpreted bytecode and must see the outer handlers at their
  real depth.
- **`--no-default-features` builds gained two unresolved-`Box` errors.**
  `jit_transfer` boxes its payload, and `Box` comes from std's prelude, which
  is absent under `no_std`. The sites are made resolvable with a guarded
  `alloc::boxed::Box` import rather than left to stand, since `lib.rs`'s
  `extern crate alloc` is itself conditional.

## 0.19.2 — 2026-08-18

### Fixed
- **Validator false-red on legal OTP-27 emission** (aion#64). The frame-size
  check in `loader/validate.rs` tracked the current stack frame *linearly*
  through the instruction stream, while frame knowledge is control-flow state.
  A shared failure island (a label followed by e.g. `badmatch {y,N}`, reachable
  only by jump from a region whose frame is large enough) that erlc places
  after *another* function's tail was checked against that tail's stale frame —
  so a legal module was rejected with "y register outside frame size". OTP-27
  erlc places such islands after foreign tails more aggressively than OTP-29,
  which is why the failure tracked toolchain, not machine. The fix makes the
  tracker honest about what it knows: every no-fall-through instruction
  (`return`, tail calls, `jump`, raises, the `*_end` family, `wait`,
  `func_info`) now resets the tracked frame to unknown, so code reachable only
  by jump is no longer judged against a frame it never had; straight-line
  frame checking is unchanged and still red on genuine violations. `trim` is
  now tracked (shrinks the checked frame), tightening a previously unchecked
  window. Downstream symptom fixed: aion's `runtime_codecs` harness hung
  15 minutes silent because the rejected module was best-effort-skipped and
  the test process crashed undef with no monitor — with this fix the module
  loads. Three regression tests pin the island, straight-line-red, and trim
  shapes.

## 0.19.1 — 2026-08-18

Dependency currency release under the estate-wide everything-on-latest word
(2026-08-18). No public API change: the dep-major moves below are all
internal — measured, not asserted (no cranelift, base64, or ecow type is
nameable from beamr's public surface; the two `JITModule` occurrences inside
public structs are private fields).

### Changed
- cranelift `0.131.2` → `0.134.3` (all five crates). Mechanical adaptation to
  the interned-`MemFlags` API (`MemFlags::trusted()/new()` →
  `MemFlagsData::trusted()/new()`), `FunctionBuilder::finalize` now taking the
  target frontend config, and the deprecated `*_imm` instruction-builder
  methods replaced by their `_s` variants — the sign-extending forms, chosen
  because they reproduce the old `Imm64` semantics byte-for-byte; no call
  site's behaviour changed.
- base64 `0.22.1` → `0.23.1` (internal to term JSON encoding).
- In-range lock refresh across the workspace (tokio 1.53.1, serde_json
  1.0.151, wasm-bindgen 0.2.127 line, crossbeam set, libc, mio, and the rest
  of the census's 16 in-range rows).
- `gleam-types` requirement `0.4.3` → `0.4.4` (see below).

### gleam-types 0.4.4
- Removed the `ecow =0.2.6` dependency: dead edge. Every `EcoString` use was
  removed from the crate's source in earlier work but the manifest edge (and
  its exact pin, minted 2026-06-09 to track upstream gleam-core's lockfile)
  survived. Zero references in the tree; compiles clean without it. Consumers
  no longer inherit a stale exact-pinned ecow from this crate. The workspace
  pin is deleted with it — a pin guarding an edge no code uses was a shield,
  not a constraint.

## 0.19.0 — 2026-08-18

The version is forced: this release carries two breaking changes (the encoder's
typed-register refusal and `resolve_imports`' return type, both below), so it
cannot ship as a `0.18.x` patch. Everything landed on `main` since `0.18.2`
rides — the AR-1 rooting arc, the `TermAccumulator` window, the B-144 `no_std`
ungating, and the two cutter's-list items ruled due at this cut.

### Added — `ProcessContext::with_accumulator`, the rooted accumulation window

`TermAccumulator` gives native code a window in which every term built is
rooted by the accumulator itself until the window closes — the shape the AR-1
rooting arc converged on after per-site reserves proved unable to defend
multi-term construction. Additive; no existing API changed. The fused
dictionary terminals (`dict_entries_to_list`, `dict_erase_all_to_list`,
`dict_keys_for_value_to_list`) landed on the same arc and reserve before they
copy, so the unreserved shape is unrepresentable at those sites.

### Removed (breaking) — the site-4 unrooted-carrier trio

`dict_get_all`, `dict_erase_all` and `dict_get_keys` are deleted, exactly as
their `0.18.x` deprecation notes announced. Each handed out an unrooted
`Vec<Term>` and trusted the caller to have reserved first; the fused terminals
above are the replacements. Zero internal callers survived to the deletion —
the act-time compile check the deprecation was designed to arm.

### Removed (breaking) — `Scheduler::spawn_process` demoted to `test-support`

`spawn_process` (and its telemetry twin `spawn_process_with_trace_context`)
enter a module at raw instruction 0, which on every loader-produced module is
the `func_info` landing pad — the process dies `error:function_clause`
immediately, a documented death-trap that had zero production callers. Both
now sit behind `cfg(any(test, feature = "test-support"))`: scaffold tests keep
them, the default public surface loses them. Use `spawn` / `spawn_in`, which
resolve an exported entry and start after the pad.

### Changed — `no_std` builds see `timer` and `replay` unconditionally (B-144)

The `timer` and `replay` modules are no longer gated on the `threads` /
`cooperative` features, so `--no-default-features` builds (including
`wasm32`) compile them; `crossbeam-queue` became a non-optional dependency in
the same motion. The `no_std` error-count ratchet moved 1039 → 1075 with the
gate's removal — a measured population change (the gate was a hole in the
count), recorded in the ratchet instrument itself.

### Fixed (feature `jit`) — a caught exception crossing a compiled function leaked one interpreter nesting per catch

**With the JIT enabled, any in-VM `catch`/`try` whose protected region crossed a
compiled function's body call leaked a nested interpreter run — every time it
caught.** No error, no log line, and the catch appeared to work.

**What happened.** A compiled function's body call is serviced by running a
FULL nested interpreter on the same process. The exception-handler stack is
process-level and had no nesting barrier, so a raise inside that nested run
could pop a handler installed OUTSIDE it, truncate the stack, and jump to that
outer handler's catch label **while still inside the nested run**. The compiled
invocation and its native frames were never returned through. Each caught
exception therefore leaked one native (Rust) nesting, unbounded, for the life of
the process.

**How it showed.** Two ways, one of them silent. The visible one: when the
process eventually died, the fatal unwind exited the leaked runs in turn and
each leaked trampoline recorded a compiled frame, so a death record that should
name the compiled function ONCE named it **K+1 times**, where K is the number of
exceptions caught earlier — a stack trace reporting a call chain that never
happened. The silent one: unbounded native stack growth on a long-running
process that catches routinely, which is the shape of a system that runs well
for hours and then dies without a cause in its own logs.

**Exposure.** Requires the `jit` feature plus one catch whose protected region
crosses a compiled body call. It is reached only after a function tiers up, so
it lands on hot paths, and only after the system has been running a while. A
suite that passes under interpretation says nothing about it.

**The fix has two halves, and both were needed.** A nesting floor on the handler
stack stops the nested run consuming a handler it cannot return to; and the
compiled-code exception return, previously an unconditional process exit,
now re-offers the exception to the handlers at the level it reached. Without the
second half the first would have converted every such caught exception into an
uncatchable process kill — a worse defect than the one being fixed. Both halves
are gated by `crates/beamr/tests/jit_nested_run_handler_leak.rs`, whose five
arms were measured red at the unfixed bytes AND red against a first-half-only
fix.

### Changed (breaking, feature `encode`) — the encoder now REFUSES modules containing typed registers

⛔ **`encode_module` returns an error instead of silently writing a broken
module.** OTP 26+ emits typed-register (`#tr{}`) operands, each carrying an index
into the module's `Type` chunk. beamr's decoder keeps the index and **drops the
table**, and the encoder emits no `Type` chunk — so every re-encoded module
carrying one had `Code` operands pointing into a chunk that was not in the
container.

**This produced no error of any kind.** beamr's own loader reads the index and
ignores the table, so such a module round-tripped through beamr byte-for-
structure perfectly and the ratchet went green; every other BEAM tool got a
dangling reference. A silent fallback wearing a success exit code is the
failure mode that hides longest, so the encoder now refuses, **naming the
offending instruction index and `Type` entry**.

**Measured scale: 55 of the 105 committed `.beam` fixtures** contain at least
one typed register — the encoder could not faithfully represent a majority of
real modules, and said nothing. This is why the refusal lands before the fix
rather than with it.

**This is a guard, not the fix.** The fix is to carry the raw `Type` chunk
bytes through `ParsedModule` and re-emit them verbatim; the refusal is what
stops the silent-success state existing in the meantime, and it disappears when
the fix lands. Downgrading typed registers to plain registers was considered and
**rejected**: `decode -> encode -> decode` structural equality is the encoder's
whole ratchet, and a lossy rewrite would spend the instrument that catches the
next regression in order to buy a green run.

Blast radius: `encode` is **not** a default feature and has no in-tree caller
outside the round-trip tests. External callers that encode OTP 26+ modules will
now get a named error where they previously got bytes no other tool could read.

### Fixed — a constant-less atom table made every `Atom::*` resolve to a WRONG name

`AtomTable::new()` built a table that seated no constants and started its index
counter at 0 — the very indices `Atom::OK`, `Atom::NIL` and the other 75
constants already occupy. The first name such a table interned took `Atom::OK`,
the fifth took `Atom::NIL`, and so on, so **every constant resolved to a real
but unrelated name**.

⚠️ **That is worse than resolving to nothing.** #98 found it as a telemetry span
reporting `code.module = "put_chars"` — a plausible name, from the right domain,
which survives the sanity check a missing name would fail. A confidently wrong
answer outlives an absent one.

`new()` now seats the constants and **there is no constructor that omits them**,
so the door is closed rather than renamed. `with_common_atoms()` remains as a
delegating alias, and `Default` — which nothing in the workspace reaches today,
measured, but which `#[derive(Default)]` would reach without looking like a
decision — is correct for free.

**Blast radius: none in production.** All 185 `new()` call sites are test code;
the scheduler builds its table with `with_common_atoms()`. This is recorded as a
fixed footgun, not a shipped defect.

The suite's only prior coverage of constant seating went through
`with_common_atoms()` — **it pinned the safe door and left the unsafe one
unpinned**, which is how this survived to be found by a telemetry span rather
than by a test. Two pins now cover the property and all three public
constructors.

### Fixed — the clippy gate never linted the `encode` module

The canon `clippy` leg ran without `--features beamr/encode`, so the entire
encoder was invisible to it — the **same blind spot** as the `tests` leg fixed
in the entry below, one leg over. It was hiding a live `vec_init_then_push`
warning introduced by that very fix. Both are corrected: the leg now passes the
feature, and the warning is gone.

⚠️ Finding a leg that skips a module is not evidence about that leg alone. The
adjacent-leg question — *does anything else compile this code?* — is the one
worth asking at the time.

### Fixed — no gate compiled the `telemetry` module at all

The canon compiled exactly **one** feature combination, and a whole module tree
escaped it. `telemetry` is enabled by **nobody**: not `beamr-cli` (default
features), not `beamr-wasm` (`default-features = false`, `cooperative` + `json`),
not the `clippy`/`tests` legs' `--features beamr/encode`, and not
dev-dependencies. Yet `#[cfg(feature = "telemetry")] pub mod telemetry;` gates
four files and **156** `feature = "telemetry"` cfg sites.

**Measured, not argued.** A deliberate type error planted in
`telemetry/spans.rs` passed **all six** then-legs — `fmt` 0, `clippy` 0,
`wasm32-check` 0, `wasm-tests` 0, `tests` 0, `blocking-call` 0 — while the
positive control `cargo check -p beamr --features telemetry` returned rc 101
naming exactly that line and nothing else.

A new `clippy-all-features` leg closes it, and its own falsifier is recorded:
that same specimen takes the new leg to rc 101 while the untouched tree is rc 0.

⚠️ **The new leg does not replace its sibling, and must not be collapsed into
it.** Every cfg has a complement: under `--all-features` all
`#[cfg(not(feature = "…"))]` code stops being compiled, and `not(feature =
"threads")` is how the cooperative runtime is selected. The canon now compiles
**three named combinations on purpose** — default-union-`encode` (the shape host
consumers build), `cooperative`+`json` with `default-features = false` (the
`wasm32-check` leg, which is the only leg that type-checks the not-`threads`
paths), and all-on (the new leg, the only leg that compiles `telemetry`).
**Three named points out of 2^14 is not "every feature combination is gated"**;
a green canon should be read as exactly those three and no more.

`--all-features` rather than an enumerated `--features a,b,c` is deliberate: an
enumerated list is a second copy of the feature set, and it drifts the first
time somebody adds a feature and forgets the gate. This way a newly declared
feature is covered by construction.

✅ **That known-red bar is now RESOLVED.** It was declared here as 1838 passed,
**4 failed**, all four telemetry-gated `scheduler::tests`, with the cause —
stale test setup or live scheduler change — explicitly unresolved. It was a
stale setup, but by a mechanism neither candidate named, and the comfortable
half of the guess was wrong too.

All four entered at `instruction_pointer: 0`, which in their module layout is
the `FuncInfo` instruction — the multi-clause dispatch **landing pad**. Since
`5bcd529` (shipped in `0.16.3`) `func_info` raises a catchable
`error:function_clause` instead of continuing, so each test killed its process
on its first instruction, before the slice ran. **`shutdown` was irrelevant**:
a probe with the shutdown call removed produced byte-identical outcomes, and a
`Return` module expecting `Exited(Normal)` failed the same way, so the fault
was never about yielding. Entered one instruction later, a looping process is
preempted and requeued exactly as asserted — **there is no scheduler fairness
defect**, and production never had one: `Scheduler::spawn_in` resolves
`label_ip` and enters after the pad.

Fixing that exposed a second one. With a live entry point the remaining test
reported `code.module = "put_chars"`, reproducibly and alone. The `test_module`
scaffold built an **empty `function_table`**, so `Module::mfa_at_ip` returned
`None` for every ip and the span fell back to `Atom::NIL` — which resolved to a
real, unrelated name because these tests build their table with
`AtomTable::new()`, a constructor that seats **no** constants and assigns
indices from 0, colliding with every `Atom::*`. The scaffold now derives its
`function_table` from its `FuncInfo` instructions, mirroring the loader.

`cargo test --workspace --all-features` is now **1842 passed, 0 failed**, and
the all-features *tests* leg is unblocked. Full record in `gate-logs/98/`.

### Changed (breaking) — `resolve_imports` returns `Vec<ResolvedImport>`, not `Vec<Option<…>>`

`resolve_imports` declared `Vec<Option<ResolvedImport>>` while **never producing
a `None`** — all four push sites wrapped in `Some`, and an import that fails to
resolve becomes a `ResolvedImportTarget::Unresolved` *variant*, not an absence.
The `Option` was vestigial, and it made a dangerous shape expressible.

**What it would have cost.** The vector is positional: entry `i` is the
resolution of `ImpT` entry `i`, and instructions name their target by that same
index. Loading ran `resolved_by_index.into_iter().flatten().collect()` before
handing the vector on, and `jit/runtime.rs` indexes the stored vector by the
original instruction index. A single `None` would therefore have shifted every
later import down one and **silently dispatched calls to the wrong function** —
no error, no crash, a valid-looking target. Validation would not have caught it:
it bounds-checks against the *unflattened* slice and never inspects `Some`.

No released version could produce a `None`, so **this fixes no observed
misbehaviour** — it removes the ability to express one. The `flatten` is gone
along with the `Option`, and the shift is now a **compile error** rather than an
invariant someone has to remember.

Callers that matched on `Option` should drop that layer; callers that already
treat every entry as present need no change.

### Fixed — modules beamr writes can now be opened by OTP's own tools

`encode_module` emitted `ImpT`, `ExpT` and `StrT` **only when they had
contents**. Our own loader treats an absent optional chunk as an empty one, so
such a module round-tripped through beamr perfectly and every in-tree test
passed. **OTP does not agree.** `beam_lib` hard-requires all three, and refuses
the file outright with `{missing_chunk, _, "StrT"}` before disassembly begins —
so a module beamr wrote could not be read by `beam_disasm`, by `beam_lib`, or by
anything built on them. That cost has already been paid once: a 7,828-module
ecosystem sweep silently skipped our module, and a skip looks exactly like a
clean result.

The required set was **measured, not assumed** — each chunk was stripped in turn
from a working module and the remainder fed to `beam_disasm` under OTP 29, with
the unstripped module as a positive control. Required: `AtU8`, `Code`, `ImpT`,
`ExpT`, `StrT`. Genuinely optional, and still conditional: `Attr`, `CInf`,
`Dbgi`, `Docs`, `FunT`, `Line`, `LocT`, `Meta`, `Type`. `LitT` stays conditional
and is *not* claimed either way — stripping it leaves dangling literal
references, making that arm a confound rather than evidence.

Effect, over all 24 sample fixtures re-encoded and fed to OTP 29 `beam_disasm`:
**0 of 24 readable before, 12 of 24 after**, with the original `erlc` bytes
reading 24 of 24 as the control.

⚠️ **This does not make every beamr-emitted module disassemblable, and the
remaining 12 failures are a different, disclosed defect.** beamr emits typed
registers into `Code` while dropping the `Type` chunk they index, so the
reference dangles and `beam_disasm` raises `cannot_disasm_instr`. The type table
is discarded at decode — `ParsedModule` has no field for it — so this cannot be
fixed by emitting a chunk and is tracked separately. The correlation is exact:
the 12 fixtures containing at least one typed register are precisely the 12 that
still fail, and the 12 containing none are precisely the 12 that now pass.

### Fixed — the encoder's test suite now actually runs in the gate

`encode` is not a default feature, and no `gates.json` leg and no CI workflow
enabled it. `crates/beamr/tests/encode_round_trip.rs` is `#![cfg(feature =
"encode")]`, so **the entire round-trip ratchet compiled to nothing** and had
never run in the battery. The `tests` leg is now
`cargo test --workspace --features beamr/encode`.

⭐ **The way this hid is worth stating.** The binary was still built and still
run; it simply printed `running 0 tests` and `test result: ok. 0 passed`. The
result-line count — 73 before, 73 after — could not tell "ran" from "ran
nothing", and `ok. 0 passed` is the same green as any other. Only the passed
count moved: **2094 → 2107**, of which just 2 tests are newly written and 11 are
pre-existing tests running for the first time.

## beamr-wasm 0.9.0 — 2026-08-18

- Rides beamr 0.19.0 (dependency spec `^0.17.0` → `^0.19.0` relative to the
  published 0.8.0) so downstream wasm consumers resolve ONE beamr per lock on
  the current line. No API changes of its own. Closes the wasm-rung
  prerequisite tracked as board row #227b: published beamr-wasm 0.8.0 carries
  `^0.17.0`, which cannot unify with any newer beamr native pin in the same
  tree.
- The wasm bindings' conversion path is built on `ProcessContext::
  with_accumulator`, which first ships in beamr 0.19.0 — the publish dry-run
  against registry 0.18.2 failed with four E0599s, which is how the floor was
  measured rather than assumed.
- Spec-history note, so the published sequence stays reconstructible: the
  in-tree manifest carried `0.18.0` (commit `83bd74d`) and briefly `0.18.2`,
  neither published. The published sequence is `0.8.0 = ^0.17.0` →
  `0.9.0 = ^0.19.0`.

## beamr-cli 0.6.0 — 2026-08-18

- Rides beamr 0.19.0 (dependency spec `^0.17.0` → `^0.19.0` relative to the
  published 0.5.0; the in-tree `0.18.0` spec was never published). No CLI
  changes of its own.

## 0.18.2 — 2026-08-12

Patch release, cut from `main`. Two fixes, both in the JIT's runtime helpers,
both of the same class: **a wrong answer returned in the shape of a right
one.**

### Fixed — compiled code now delivers the messages it sends

`jit_send_message` implemented exactly one destination — the sending process
itself — and let every other destination fall through while returning the
message term, which is what a successful `!` evaluates to. See the advisory at
the top of this file for the affected range and how it was measured.

The helper now reaches the full set of destinations through the `services`
pointer already installed in `JitRuntimeContext` around every native call from
compiled code: self-sends go through the interpreter's own `send_to_self`, and
local sends through the same `LocalSendFacility` block the interpreter uses —
**shared, not reimplemented**, because a second copy of a dispatch is a second
copy that can drift. Remote destinations go to the distribution facility.

Destinations that genuinely cannot be served park the interpreter's *own*
`ExecError` in a new `Process::jit_exec_error` and abort: a non-pid destination
raises the same `badarg` the interpreter raises, and a remote send with no
distribution raises the same `noconnection`. `call_native` checks that field
**before** the deopt status, so an abort can never be re-entered as a restart
and a partially-completed send can never be replayed. `Send` is deliberately
**not** added to `is_runtime_deopt_capable`: the admission guard is untouched
and no function that compiles today stops compiling.

The self-send arm additionally ticks the sender's logical clock and consults
the replay driver, both of which it previously skipped — so compiled and
interpreted self-sends no longer diverge under record/replay.

### Fixed — a compiled stack frame named the wrong module, and invented a line

Both stacktrace renderers read a frame's function and arity from its recorded
`mfa` and then discarded that same `mfa`'s module atom, taking the module name
from the frame's pinned module instead. For interpreted frames the two agree by
construction and the discard was invisible. For a compiled frame they are
different things — the pinned module is only wherever the process happened to
be positioned — so a crash report could name **a function that does not exist
in the module the frame claims**. Resolution now lives in one place,
`RawStackEntry::identity`, and prefers the recorded `mfa`.

The two renderers had also drifted: the in-VM one skipped line resolution for
compiled frames, the crash-report one did not, and a compiled frame's
instruction pointer is a placeholder zero. A compiled frame now reports no line
**structurally**, rather than resolving one from a placeholder and relying on
what that placeholder happens to produce.

🔴 **CORRECTION, recorded because an earlier draft of this entry overstated
this half.** That draft said the crash-report renderer carried "the module's
first line-table entry" as the frame's line. **It does not.** `line_at_ip`
binary-searches the line table and subtracts one from the insertion point, so
`ip = 0` yields the first entry only if a line marker sits at **exactly** ip 0
— and no measured module has one.

**Stated with its ground, because the ground is bounded.** Across all 24 sample
`.beam` modules, decoded through beamr's own loader, the instruction at ip 0 is
a `label` and the first `line` marker is at ip 1 — without exception; and the
loader's own producer-path test has asserted `line_at_ip(0) == None` on a built
module throughout. That is **the observed behaviour of the current compiler
over the measured set, not a rule beamr enforces.** The loader validates
nothing about prologue ordering, so nothing here forbids some future producer
emitting a `line` at ip 0. ⇒ **In every module measured, `line_at_ip(0)` is
`None` and the placeholder produced no line at all** — and the claim is not
extended past that population.

That bound is also the honest justification for keeping the guard: it protects
against a producer that does not behave the way today's does. A new loader test
gates the assumption directly, so if that day comes, the fixtures depending on
it fail loudly instead of quietly becoming fiction.

⇒ **The observable effect of this half of the fix is nil.** It removes a
correctness-by-coincidence — nothing forbids a producer emitting a line marker
at ip 0, and the guard no longer depends on that — but it does not stop a wrong
line being printed, because none was. **The module splice above is the half
with observable effect**, and that half is confirmed against a real crash
report from a production `0.17.0`. This claim was refuted by an external
reviewer reading the shipped `0.17.0` bytes, then re-derived independently
here; it is corrected in place rather than quietly dropped, because a changelog
that overstates one fix teaches its readers to discount the rest.

### Known — the frame builder has a third defect, open and unfixed

`function_table` records the ip of each `func_info` instruction while
`line_table` records the ip of each `line` instruction, and a BEAM function
prologue is `label → line → func_info`. There is therefore **one instruction
position per function** (every function after the first) at which the line
table has already advanced to the next function's head line while the function
table still reports the previous function — so a frame resolved at that
position pairs one function's name with the next one's line. Measured by
decoding a shipped module through beamr's own loader.

It is disclosed rather than fixed because **no path has been shown to place a
recorded position there**: a `line` instruction cannot raise, the interpreter
writes the position after dispatch rather than before, and the one instruction
past a tail call is the following `label`, not its `line`. Two candidate
firing paths were proposed during this work and both were refuted by
measurement. The fix would move a function's recorded region to begin at its
prologue, which changes `Module::mfa_at_ip` and therefore `Process::current_mfa`
— not a change to make on a mechanism whose trigger is unnamed. Note that
`mfa_at_ip`'s own doc-comment asserts the invariant this defect violates.

### Not fixed here

The accumulator-rooting class (AR-1) disclosed in the `0.18.1` entry remains
**open and unfixed** in this release: 17 verified crossings, no JIT required.
This release does not touch it and does not narrow it.

## 0.18.1 — 2026-08-10

Patch release, cut from `main`. Patch-level because the compatibility claim
was **measured, not argued**: the entire production delta is three modules,
all `pub(crate)` in `jit/`, with zero re-exports out — public Rust API
surface UNCHANGED (measurement commit `799730c`; `cargo public-api` refused
on a rustdoc-JSON version mismatch, which is recorded as a refusal and not
as a negative, and two other instruments each with a positive control were
used instead).

### Fixed — RF-006: the JIT GC-rooting class

The "Known remaining JIT sites" class disclosed in the `0.16.3` and
`0.17.0` entries below is fixed. All sites reachable under the `jit`
feature (on by default):

- `jit_bs_start_match` roots the source binary `Term` across the
  match-context allocation instead of capturing it at entry and writing a
  potentially stale copy after (wall:
  `start_match_source_survives_forced_collection`, RED at the parent by
  design in this branch's history).
- Helper-held terms are rooted across collecting allocations throughout
  `jit/runtime.rs`, `jit/runtime_map.rs` and `jit/runtime_binary_match.rs`;
  the new `alloc_words_rooted` **refuses** rather than returning a stale
  term when rooting cannot be established.
- `jit_bs_get_binary` ProcBin extraction **shares O(1) again** and roots
  the forwarded parent — the sound fix, replacing the earlier
  copy-workaround (walls:
  `bs_get_binary_procbin_extraction_shares_forwarded_parent`,
  `bs_get_binary_procbin_source_box_referent_survives_forced_collection`).

Evidence, re-run at this release's rebased base rather than carried from
the pre-rebase branch: the process-bearing probe suite
(`native/stdlib_stubs/gc_rooting_tests.rs`, 17 tests, each asserting a
collection actually fired via `old_used() > 0`) is green, and all three
committed mutations (M1/M2/M4 under
`docs/design/beamr/briefs/evidence/review-23-07/rf-006/mutations/`) were
applied, went RED with their own wall in the failure list, and were
reverted — artifacts in
`docs/design/beamr/briefs/evidence/review-23-07/rf-006/runs/rebase-0.18.1/`.
The fix lane's stale-`Term` sweep records **zero remaining REAL-verdict
sites under `crates/beamr/src/jit/`** (`rf-006/sweep/verdicts.json`).

The advisory paragraph that named RF-006 as this class's owner ("not fixed
in any version cut to date") is removed in this same commit, as its own
text required.

### Correction, forward-only — "ABI-level" was an unmeasured claim, now refuted

The `0.17.0` and `0.16.3` entries below describe the sound fix as "an
ABI-level change owned by RF-006, not backportable in a patch". Those
entries are released history and stay as written; the correction lives
here: **measured at the fix, the change is not ABI-level and does not
touch the public API** — which is why this release is `0.18.1` and not
`0.19.0`. The earlier sentences carried a compatibility claim that had
never been measured; this entry replaces it with a measured one.

### Open and disclosed — the accumulator-rooting class (AR-1)

This release fixes the JIT rooting class and **nothing else**. The
native-BIF accumulator-rooting class described in the advisory at the top
of this file — 17 verified crossings, one shape, **no JIT required**,
present in every released version including this one — remains **OPEN**,
with no fix designed. Its audit, named-site disposition ledger, control
fixtures and pre-registered landing gate ship in this release's tree
(`docs/design/beamr/briefs/AR-1-LANDING-GATE.md`) so the class is sized in
public rather than known privately. Four further `UNRULED-PRERESERVE`
sites are recorded there as unruled, not safe. **This entry must not be
read as "the memory-safety fix": a release that fixes one class while a
sized class stays open says so in the same breath.**

## 0.18.0 — 2026-08-09

Minor release, cut from `main`. **Breaking, deliberately.** Twelve public
entry points across four families silently substituted an empty native
surface — an empty `BifRegistryImpl`, or a `NativeServices::default()`, or
both — when the caller did not supply one. That default cost two multi-seat
attributions (the 2026-07-18 gc_bif incident is the sanctioned one), was
already described in three long rustdoc footgun sections, and had already
provoked a downstream wrapper (`frame_core::composition::compose_scheduler`)
built specifically to route around it. **Documentation was tried, a
downstream workaround was tried, and the second incident still happened.**

**"No natives" remains a supported configuration.** Four shipped production
consumers use it, and nothing here takes it away. What changes is that it is
now a value you write down: `NativeBifs::none()` at the call site, not an
empty registry conjured by a constructor that did not mention one. The sin
was the default, not the emptiness.

The error the wrong composition produces changes with it. `GuardBifUnavailable`
used to name the module that was executing — which is never the module at
fault — and its hint said "native BIF registry has no entry" without saying
*which* registry, of which there are two: the one the loader resolved imports
against, and the one the scheduler carries. It now reports which of the two
was reachable at the refusal, and points at construction rather than at the
executing module. It still does not claim your registry is empty, because
`BifRegistry` exposes no emptiness predicate and the two registries can
legitimately disagree — a claim we cannot substantiate is how the first
incident accused an innocent component, and we are not repeating it in a new
direction.

### Removed (breaking)

- **`interpreter::run`** — executed with `NativeServices::default()` and no
  module registry. Migrate to
  `run_with_native_services(process, module, &ModuleRegistry::new(), &NativeServices::default())`;
  the substitution is behaviour-identical (every consumption of the optional
  registry is `.and_then(|r| r.lookup(..))`, so an empty registry and no
  registry are indistinguishable).
- **`interpreter::run_with_registry`** — carried a module registry and
  defaulted the services. Migrate to
  `run_with_native_services(process, module, registry, &NativeServices::default())`.
- **`interpreter::run_with_timer_services`** — filled one of `NativeServices`'
  34 fields and nulled the registry. Zero callers estate-wide at removal.
  Migrate to `run_with_native_services` with a `NativeServices` carrying your
  `timers`.
- **`interpreter::opcodes::dispatch_with_timer_services`** — same shape at the
  opcode seam. Zero callers estate-wide at removal. Migrate to
  `dispatch_with_services`.

`interpreter::run_with_native_services` is unchanged and is now the single
interpreter entry point.

### Changed (breaking)

- **`Scheduler::new(config, module_registry)` → `Scheduler::new(config, module_registry, natives)`.**
  Pass `NativeBifs::registry(bifs)` to keep executing bytecode, or
  `NativeBifs::none()` to declare that this scheduler resolves no natives.
- **`Scheduler::with_services(config, services, module_registry)` → `(config, services, module_registry, natives)`.**
  Same mapping. Note that this constructor's own docs used to redirect
  embedders *here* from `Scheduler::new`, and it defaulted the registry too —
  the prescribed migration target was itself in the defect class. It is no
  longer, and the redirect now points at `with_services_and_code_server`.
- **`Scheduler::new_replay(config, log)` → `(config, log, natives)`.**
- **`Scheduler::new_replay_with_registry(config, module_registry, log)` → `(config, module_registry, log, natives)`.**
  The "registry" this constructor names was always the MODULE registry; the
  fourth argument is the native one.
- **`ReplayDebugger::new(process, module)` → `(process, module, services)`** and
  **`ReplayDebugger::with_snapshot_granularity(process, module, granularity)` → `(process, module, granularity, services)`.**
  Pass `NativeServices::default()` for the previous behaviour. The builders
  `with_registry` and `with_native_services` are unchanged and still override.
- **`opcodes::dispatch(.., registry)` → `dispatch(.., registry, services)`** and
  **`opcodes::dispatch_with_receiver(.., receiver, registry)` → `(.., receiver, registry, services)`.**
  Pass `None` for the previous behaviour. `Some(&services)` now behaves exactly
  as `dispatch_with_services` does with the same bundle.
- **`ExecError::GuardBifUnavailable` gains a `runtime_natives` field**
  (`RuntimeBifRegistryState::{Absent, Unwired, Wired}`). Matchers using
  `{ .. }` are unaffected; exhaustive field destructuring needs the new name.
  A field rather than a new `ExecError` variant, so exhaustive `ExecError`
  matches (e.g. `beamr-wasm`'s reason mapper) keep compiling.

Constructors NOT changed: `Scheduler::with_services_and_code_server`,
`Scheduler::replay_with_services_and_code_server`, `Scheduler::with_code_server`,
`Scheduler::with_code_server_and_policy`, `WasmScheduler::new`,
`opcodes::dispatch_with_services`, `interpreter::run_with_native_services`.
All of them already required the caller to supply the native surface; they are
the shape the rest has been moved onto.

### Changed

- **The guard-BIF refusal points at construction instead of accusing the
  executing module.** `GuardBifUnavailable` now renders which of the three
  runtime registry states applied (`Absent` — no services bundle reached the
  dispatch; `Unwired` — a bundle with no registry; `Wired` — a registry was
  present) and carries a fixed pointer naming the constructor family. Which
  family it names follows the build: `NativeBifs::none` / `NativeBifs::registry`
  under `threads`, and `WasmScheduler::new`'s `bif_registry` argument under
  `cooperative`, where `NativeBifs` does not exist. That build is the one
  `beamr-wasm` ships and the one whose reason mapper hands this line to
  JavaScript, so a pointer at a threads-only type would point at nothing
  there. It does
  NOT assert that your registry is empty: imports bind at LOAD time against the
  loader's registry, `Module`/`ResolvedImport` record no provenance, and the two
  registries can differ, so that assertion could accuse a correctly-composed
  caller. The `Deferred` hint now says "the LOAD-TIME native BIF registry",
  naming which of the two it means, and stamps its other half the same way —
  "the LOAD-TIME module registry did not hold the target". Both conjuncts are
  load-time facts: imports resolve exactly once, at load, while a `Deferred`
  import is late-bound against the live module registry on the call path, so a
  present-tense "the target module is not loaded" would be a claim about
  runtime state the refusal's mint point cannot see. Note what `Wired` does and
  does not tell you: a scheduler declared `NativeBifs::none()` reports `Wired`
  too, because `none()` wires an empty registry. On the scheduler path `Wired`
  is the value you will almost always see; the discriminating information is the
  pointer, not the state.

### Downstream

Estate consumers pinning `beamr = "0.16.3"` are unaffected until they bump.
On bumping, four shipped production call sites need one argument each —
haematite `db/startup.rs:343`, liminal `conversation/actor.rs:289`, liminal
`channel/supervisor.rs:152`, liminal-server `connection/supervisor.rs:1091`
— plus eleven test/example sites. `frame_core::composition::compose_scheduler`
already constructs correctly; its module doc cites `0.15.1` coordinates and
the retired `InvalidOperand("guard bif native import")` shape, and frame should
decide whether the wrapper is still earning its keep.

`beamr-cli` and `beamr-wasm` keep their own versions (`0.5.0` / `0.8.0`) — no
source change in either crate — but both move their `beamr` dependency pin to
`"0.18.0"`, which a path dependency requires to resolve.

## 0.17.0 — 2026-08-07

Minor release, cut from `main`. **The version is forced, not chosen:**
`Scheduler::watch_exit` is additive public API and the `spawn_link_dirty`
removal is breaking, so a `0.16.4` cut from `main` would ship a version
number that misdescribes its own contents.

**This repository has two release lineages, and that is a standing property
of it rather than an anomaly to be rediscovered.** The minor line lives on
`main`. Patch releases on the `0.16.x` line are cut from the `0.16.3`
release point, which is tagged `v0.16.3` and is **not an ancestor of
`main`** — `main` carries 0.16.3's memory-safety fixes as forward-ports,
established file-by-file rather than assumed. A fix that has to ship as a
`0.16.x` patch is cut from that lineage, never from `main`. Consequently
**whether a base carries the 0.16.2 fixes is a commit-ancestry question,
not a version comparison** — see **Pinning** in the advisory above.

#### Verifying the forward-ports yourself

**Every identity-based carriage test reports these fixes as ABSENT from
0.17.0, and each one is wrong.** A forward-port is the same change under a
different patch, so commit SHA, ancestry, `git cherry` and `git patch-id` all
answer "missing" — `git cherry` marks every 0.16.3 fix commit `+`, which reads
plainly as *the memory-safety fixes did not make it*. **Only content answers.**
"Established file-by-file" records that we checked; it hands you nothing to
check with, so here are the markers:

| lane | file (under `crates/beamr/src/`) | marker | at `67f89c4` | at `v0.17.0` |
|---|---|---|---|---|
| 3 | `jit/runtime_binary_match.rs` | `Own the bytes` | 0 | 1 |
| 3 | `jit/runtime_binary_match.rs` | `let bytes = bytes.to_vec();` | 0 | 1 |
| 3 | `jit/runtime_binary_match.rs` | `Advance the position BEFORE the allocation` | 0 | 1 |
| 1 | `native/gate3_bifs/mod.rs` | `to_vec()` | 0 | 1 |
| 1 | `native/stdlib_stubs/misc_bifs.rs` | `to_vec()` | 0 | 1 |
| 1 | `native/stdlib_stubs/uri_bifs.rs` | `to_vec()` | 0 | 1 |
| 1 | `native/stdlib_stubs/string_bifs.rs` | `to_vec()` | **2** | **7** |
| 2 | `native/etf_bifs.rs` | `to_vec()` | 0 | 2 |

```sh
git show <rev>:crates/beamr/src/<file> | grep -c -F '<marker>'
```

⚠️ **COMPARE THE TWO COLUMNS — DO NOT TEST FOR PRESENCE.** `string_bifs.rs`
is **2 → 7**, not 0 → n: it already contained the idiom before the fix, so a
presence test passes at the pre-fix state and reports carriage that is not
there. That row is the reason this table has a left-hand column at all.

The counts are a *carriage* check, not a proof of equivalence — they tell you
the forward-port arrived, not that it is byte-identical to the `0.16.x` patch.
For the stronger claim, compare the blobs. **All six files named above are
byte-identical between `v0.16.3` and `v0.17.0`:**

```sh
git rev-parse v0.16.3:crates/beamr/src/<file> \
              v0.17.0:crates/beamr/src/<file>   # two identical object ids
```

`beamr-cli` moves to `0.5.0` and `beamr-wasm` to `0.8.0` in the same commit;
both pin `beamr = "0.17.0"`. `gleam-types` is deliberately not bumped — no
commits since its version was set.

### Added

- **`Scheduler::watch_exit` — notification-only, per-pid, one-shot exit
  watches** (`ExitWatch`, `ExitWatchState`). The scheduler-to-waiter exit
  notification liminal's «CHANNEL-REPLY-EVENT-RACE» (W4 F7) was blocked
  on: unlimited concurrent watches deliver `(pid, reason)` by value
  without touching the exclusive outcome-claiming
  `subscribe_exit_events` slot or `take_exit_outcome`'s exactly-once
  semantics. Registration answers are typed
  (`Live` / `AlreadyExited` / `NoRecord`); already-dead answers are
  served from the durable outcome record, so they survive both
  legacy-tombstone eviction and outcome consumption; dropping a watch
  deregisters it. Fires are appended strictly after outcome
  installation and the existing event publication, preserving the
  ordering aion's drainer errors on.
- **`Scheduler::replay_with_services_and_code_server`** — the replay
  counterpart of `Scheduler::with_services_and_code_server`, carrying
  services, module registry, atom table and a populated BIF registry into
  replay mode. `new_replay` and `new_replay_with_registry` both default
  the native BIF registry to an EMPTY one, so a replayed module importing
  `erlang:*` refuses process-fatal at its first guard-BIF; this is the
  registry-carrying path their docs now direct embedders to.

### Changed

- **`beamr replay` refuses instead of reprinting the recorded
  transcript.** It previously loaded the log, read its stored
  `cli_result`, and returned that recorded stdout and exit code as the
  replay's own result — so a log recorded against a working build still
  reported success after the build stopped producing that output, and a
  stored transcript was emitted as though it were a reproduction. It is
  now a loud `CliError::ReplayCannotReproduce` at exit 1. The log format
  records no module identity, entry point, or runtime arguments, so there
  is nothing to re-execute: **replay now fails for every log**, which is
  the honest state of the feature rather than a regression in it. The
  recording half is unwired (`ReplayRecorder` has no callers, `record`
  writes zero events), so no round trip was working to break.

  **Why it survived: a test was endorsing it.**
  `record_then_replay_fixture_preserves_stdout_and_exit_code` asserted
  that the replayed result equalled the recorded one — but both sides
  came from the same stored string, so the assertion held no matter what
  the runtime did. It was green, and it could only ever have been green.
  **A test that cannot fail is not weak coverage; it is an active
  endorsement of the defect it covers.** That test is deleted and
  replaced by three walls that assert the refusal and, in each case, that
  the refusal does not smuggle the recorded transcript back out through
  the error message.

  **What would restore replay:** a log format that records the code
  identity it recorded against — module identity or module bytes, the
  entry point, and the runtime arguments — so a replay scheduler can
  reload that code and re-execute it with the driver supplying the
  recorded nondeterministic decisions. This is a refusal pending a format
  that can carry those fields, not an abandonment of record/replay; the
  replay-side driver, the step debugger and the on-disk format are all
  present and unchanged.
- **`beamr replay` refuses `--dir`**, mirroring the existing `compile`
  guard. Module context belongs to the recording; accepting it from a
  replay-time flag would be a supported way to replay against different
  code than was recorded.

### Removed (breaking)

- **`Scheduler::spawn_link_dirty`.** A behavioral `spawn_link` alias since
  2026-07-20 (`65f499c`): dirty scheduling is a property of the NATIVE
  ENTRY, not the process, so the method's dirty hint never did anything.
  Zero consumers estate-wide at removal (the last, aion `handle.rs:330`,
  migrated to `spawn_link` at the 2026-07-23 family convergence).
  Migration: call `spawn_link` and register dirty natives
  (`BifRegistryImpl::register_dirty`) — behavior is identical. Per-entry
  dirty dispatch and real-link coverage stay pinned by
  `spawn_link_dirty_dispatch_is_per_entry_on_the_linked_path` in
  `tests/dirty_scheduler.rs`.

### Known remaining JIT sites (still open at this release)

**`0.17.0` does not fix the JIT rooting class, and does not close it.** It
makes the `0.16.2` and `0.16.3` memory-safety fixes available on the minor
line. What remains is **one named site and two open classes**, carried
forward from the `0.16.3` entry unchanged in substance. **The two classes are
not enumerated** — each covers an unbounded set of call sites, so the list
below is a description of the exposure, not a count of it. They are restated
here rather than left as a back-reference, because a reader upgrading to
`0.17.0` should not have to read a superseded entry to learn what is still
exposed.

⚠️ **AND `jit` CANNOT BE DISABLED IN ANY BUILD THAT RETAINS `threads` —
CORRECTING WHAT EVERY PRIOR ENTRY IMPLIED.** Earlier disclosures called these
sites reachable "only under the optional `jit` feature", which reads as an
available mitigation. It is not one. The manifest declares
`jit = ["std", "threads", …]`, but `scheduler/mod.rs` and the whole
`scheduler::supervision_integration` module are gated on `threads` while
referencing `crate::jit`, so:

```
cargo check -p beamr --no-default-features \
  --features std,threads,net,fs,embedded,readiness      # 7 errors, unresolved crate::jit
cargo check -p beamr --no-default-features \
  --features std,threads,net,fs,embedded,readiness,jit  # clean
```

Disabling this class therefore requires giving up `threads` as well. **That
is a defect under repair, not an intended property.** It is not new in
`0.17.0` — the coupling dates to `106f91d` and is present at `v0.16.3` and at
`67f89c4`. It is tracked for repair together with a gate that builds the
configurations these notes describe. **Until then, assume a threaded beamr
carries the JIT surface.**

The sound fix for the rooting class itself requires GC rooting of JIT helper
arguments and locals — an ABI-level change **owned by RF-006**, not yet
released in any version.

- `jit/runtime_binary_match.rs`, `jit_bs_start_match`: the source binary
  Term is captured before the match-context allocation and written into
  the new box after it — if that allocation collects, the stored source
  Term is stale (the same class as the fixed `bs_get_binary` sibling).
- Helper-argument staleness class (`jit/runtime*.rs`): raw Term arguments
  and match-context pointers held in Rust locals are not GC roots
  (`ensure_space` forwards x-registers only); any helper whose allocation
  collects may afterwards use a stale value. The consumers fixed in
  `0.16.3` no longer cross; the rest of the surface is audited under
  RF-006.
- Accumulated-results class (RF-006 finding F3): native loops that
  accumulate result Terms in Rust vectors root only the allocator's call
  arguments, so a second mid-loop collection can leave earlier accumulated
  Terms stale. Not the audited `as_bytes` class; tracked with the same
  rooting work.

## 0.16.3 — 2026-07-29

Memory-safety patch: silent borrow-across-alloc corruption fixes,
backported onto the 0.16.2 release point (`67f89c4`; 0.16.2 has no git
tag and is pinned by commit — 0.16.3 is tagged `v0.16.3`). Every fix landed
red-first per consumer against the real hazard geometry. Scope of
record: the as_bytes audit
(`docs/design/beamr/briefs/evidence/aion-encode-gc-defect/asbytes-sweep/AUDIT.md`
on the main line) through AMENDMENT 1 (main `7e56073`: site 12 +
per-consumer verdict unit) and AMENDMENT 2 (main `92a9d2e`: the
`uri_string` error-detail parse-path consumer is SAFE — it is the one
originally listed consumer NOT fixed here, because it never crosses).
Red/green evidence for this branch: `…/aion-encode-gc-defect/fix-0163/`.

### Fixed

- **Nine mechanical `as_bytes` borrow-across-alloc crossings**:
  `erlang:'++'/2` (the `Binary ++ []` arm), `binary:part/3`,
  `uri_string:parse/1` (multi-slice), the `uri_string` error-detail
  binary (`dissect_query` error path), `string:trim/2`,
  `string:split/3`, `string:find/2`, `string:pad/4` (early-return arm),
  and `string:slice/3`. An inline (≤ 64-byte) input binary whose bytes
  were read inside a collecting allocation was silently replaced with
  zeros whenever that allocation triggered a GC — the young region is
  zero-filled on reset; no error, no crash. Fix: own the bytes before
  the allocating call.
- **`erlang:binary_to_term/1,2`**: the source-binary borrow was held
  across the entire ETF decode recursion, which allocates throughout;
  a collection mid-decode zeroed the source under the decoder. Fix: the
  bytes are copied at BIF entry, before the recursion.
- **`jit_bs_get_binary` — three consumers** (`jit` feature): the
  extracted-slice borrow crossed the collecting allocation (inline
  sources returned zeros); the ProcBin arm wrote a pre-allocation
  source-Term capture into the new sub-binary after a possible
  collection (stale parent referent — result unreadable); and the
  position write-back went through the pre-collection match-context
  pointer — a wild read-modify-write into reallocated memory, observed
  corrupting the freshly allocated result. Fix: the helper owns its
  bytes, allocates uniformly (the ProcBin sub-binary arm is removed),
  and advances the position before the allocation.

### Changed

- JIT `bs_get_binary` extraction over refcounted (> 64-byte) sources now
  copies the extracted range instead of building an O(1) shared
  sub-binary; extraction loops over large sources go O(len) per step.
  Restoring the sharing requires real GC rooting of helper-held terms —
  RF-006 material.

### Added

- Thirteen hazard walls pinning every fixed consumer under forced
  collection geometry (red-first, committed red before each fix), plus
  two mutation-proven tripwire walls: the mailbox bump-only fact
  (message copy cannot collect; `HeapFull` surfaces as `SendError`; a
  collecting reroute cannot compile at the current signature) and the
  gate3 `binary_part` owned-copy fact.

### Correction to the 0.16.2 and 0.16.3 advisories

Those entries describe the remaining JIT sites as *"reachable only under the
optional `jit` feature"*. **The word "optional" was wrong, and it was wrong
in the direction that matters: it offered a mitigation that does not exist.**

`jit` cannot be disabled in any build that retains `threads`. The manifest
declares `jit = ["std", "threads", …]`, but parts of the scheduler are gated
on `threads` while referencing `crate::jit`, so a build with `threads` and
without `jit` does not compile. **This is true of 0.16.2 and 0.16.3 exactly
as published** — it is not a regression introduced here, and consumers of
those versions have no configuration that removes the class.

The available mitigations are: upgrade, or drop the `threads` feature.
**Disabling `jit` alone is not one of them, on any released version.**

### Known remaining JIT sites (disclosed; ships by project-lead ruling)

The sound fix requires GC rooting of JIT helper arguments and locals —
an ABI-level change owned by RF-006, not
backportable in a patch. All are reachable under the
`jit` feature (on by default) via compiled `bs_*` instructions — see the
correction immediately above regarding the word *optional*, which this
paragraph originally carried.

- `jit/runtime_binary_match.rs`, `jit_bs_start_match`: the source
  binary Term is captured before the match-context allocation and
  written into the new box after it — if that allocation collects, the
  stored source Term is stale (the same class as the fixed
  `bs_get_binary` sibling).
- Helper-argument staleness class (`jit/runtime*.rs`): raw Term
  arguments and match-context pointers held in Rust locals are not GC
  roots (`ensure_space` forwards x-registers only); any helper whose
  allocation collects may afterwards use a stale value. The consumers
  fixed above no longer cross; the rest of the surface is audited under
  RF-006.
- Accumulated-results class (RF-006 finding F3): native loops that
  accumulate result Terms in Rust vectors root only the allocator's
  call arguments, so a second mid-loop collection can leave earlier
  accumulated Terms stale. Not the audited `as_bytes` class; tracked
  with the same rooting work.

## 0.16.2 — 2026-07-23

Memory-safety patch: the two critical findings from the 2026-07-23 external
review (docs/REVIEW-23-07.md), fixed red-first and torn PASS before release.

### Fixed

- **C1 — GC refc-release walk could free foreign memory.** The release walk
  inferred object type from `word[0]`, but a headerless cons whose head is
  atom `false` (encoding 0x19) is indistinguishable from a `BoxedTag::ProcBin`
  header — the walk then executed `Arc::from_raw` on a heap-cons payload:
  arbitrary free + heap corruption at the next minor GC. Fixed structurally:
  every allocation now records an `AllocKind` at allocation time and all three
  release walks (minor young-region, process-death `release_all`, major
  compacted-sources) visit only allocations marked refcounted. `word[0]`
  inference is retired entirely; the fail-safe direction is documented at the
  type (a missed mark can only leak, never free foreign memory). A
  completeness sweep marked five additional refcounted-into-process-heap
  paths the review had not named (mailbox proc-bin delivery, tcp active +
  closed socket messages, udp, io results, jit sub-binary extraction), each
  pinned by a fail-first leak test. The debugger's heap census gets an
  explicitly inspection-only unfiltered walk, documented never-for-release.
- **C2 — ETS stored borrowed caller-heap terms.** `insert` kept `Term`s
  pointing into the inserting process's heap, so a post-insert GC (or process
  death) left tables reading freed/moved memory. ETS now deep-copies terms
  into table-owned storage on insert (`ProcBin`/`SubBinary` flattened to
  table-owned inline bytes — tables hold no Arcs) and copies out to the
  caller's heap on read. Map keys own their own copies (`OwnedEtsKey` /
  `OwnedTermKey` with structural `Borrow`, so probes never copy).

### Caveat

- `EtsTable`'s public `lookup`/`tab2list` signatures changed — required by
  the soundness fix (bare `Term`s in the trait contract WERE the bug).
  Technically breaking inside a patch release; no known external
  implementors or callers (verified across haematite, liminal, frame, aion,
  beamr-wasm — all touch only `OwnedTerm`/`copy_term_to_ets`, unchanged).
  Precedent: Rust's soundness-fix policy (RFC 1122).
- ETS round-trips now flatten large binaries inline: Arc sharing is lost
  through insert/lookup, a memory cost on big-binary tables. Documented in
  code; revisit candidate for the 0.17 window.

## 0.16.1 — 2026-07-23

### Added

- `Scheduler::spawn_native_trap_exit` and `Scheduler::spawn_native_link_trap_exit`:
  native processes spawned with `trap_exit` set BEFORE they are made runnable —
  the native mirror of bytecode `spawn_trap_exit`. Closes the spawn-then-set
  window in which host-side `set_trap_exit` returns `NoCaller` for a freshly
  spawned native that is transiently `Executing` mid-first-slice (`NoCaller`
  conflates that with a truly-dead process, so callers could not retry
  honestly). Existing `spawn_native`/`spawn_native_link` behavior is
  byte-identical (they delegate with the flag off; the `Process` default is
  `false`). Found as a once-per-battery race in liminal's subscriber spawn on
  high-core hosts; consumers using spawn-then-`set_trap_exit` on natives
  should migrate to the new entry points.

## beamr-wasm 0.7.0 — 2026-07-23

- Rides beamr 0.16.0 (dependency spec `0.15.0` → `0.16.0`) so downstream
  wasm consumers resolve ONE beamr per lock. No API changes of its own.

## 0.16.0 — 2026-07-23

The gleam-on-beamr enablement release: a real `gleam_otp` 1.2.0 actor
(genuine gleam-built beams) starts, takes casts, and answers a synchronous
`actor.call` on beamr. Four latent interpreter defects were surfaced by that
spike's refusal to fake past an `undef` and are fixed here; none was reachable
by any consumer's production path on 0.15.x (verified by grep-and-trace plus a
disassembler census over every consumer's loaded bytecode), but all four sit on
the hot path the moment `gleam_erlang`-based code loads.

### Added

- `proc_lib:spawn_link/1` BIF over the existing fenced closure-spawn facility
  (admission honored; a bare `run()` without a scheduler refuses with a typed
  error rather than pretending).
- `receive ... after infinity` — the `infinity` atom now selects an unbounded,
  timer-free wait (previously `badarg`), matching `wait` semantics with no
  polling construct.
- Multi-clause functions admit to the JIT: the admission guard's
  effect-before-deopt analysis is now CFG-sensitive (forward may-reach
  dataflow over the slice block graph, union join, fixpoint) instead of
  linear-slice-order, so mutually exclusive clause exits no longer
  false-positive. A single blocking receive is now admissible under
  path-sensitivity, with a positive differential through the demand path.
  The slicer retains the `func_info` prelude and lowers `FuncInfo` as a
  DEOPT terminal.
- `docs/design/beamr/probes/raiser-scan/` — rerunnable no-match raiser census
  instrument (beam_disasm scanner + probe fixtures) used as this release's
  check over consumer bytecode; positive-control verified.

### Fixed

- `erlang:send/2` silently dropped every cross-process local send (the
  ignored `send_to_attached_self` return). It now routes through the
  `LocalSendFacility` exactly like the `send` opcode — slot-locked delivery,
  sender clock ticked, replay-valid. Anything driving `erlang:send/2` from
  loaded bytecode (gleam `process.send` compiles to it) was affected.
- `func_info` set the current MFA and fell through — a multi-clause no-match
  re-dispatched forever, spinning a scheduler core. It now raises catchable
  `error:function_clause` (bare atom, BEAM semantics), watchdog-proven
  terminating.
- `if_end` raised `error:{if_clause, []}` where BEAM raises the bare atom
  `if_clause` — a loaded `catch error:if_clause` failed to match and the
  process died where BEAM recovers. Bare atom now; the unit test that pinned
  the wrapped shape is re-pinned to the true one.
- `gleam_erlang_ffi:demonitor/1` rejected boxed references (stale
  small-int-only parse — the same class as the 0.15.4 monitor fix). Dual
  parse now: boxed `ReferenceRef` first, legacy small-int fallback.
- JIT: a reachable deopt-after-side-effect divergence through the wired
  demand path (RecvMarkerReserve) is guarded — the whole class is rejected
  by the effect-reachability analysis above, with the replay probe green.

### Removed

- **BREAKING:** the native `gleam_erlang_ffi` selector shadow
  (`register_selector_bifs` and the `selector_ffi` module) is retired. It was
  pinned to a pre-1.3 `gleam_erlang` selector protocol and silently returned
  wrong shapes under 1.3.x; the selector family is now served by the loaded
  `gleam_erlang_ffi.beam` bytecode shipped with the user's `gleam_erlang`.
  With no bytecode loaded the family fails as an honest, catchable
  `error:undef` (pinned by regression test) instead of a silently-wrong
  value. Embedders that registered the shadow should simply drop the call —
  the maps BIFs the bytecode path needs are all present.

## 0.15.4 — 2026-07-18

### Added

- Additive registry API: `BifRegistryImpl::replace_existing(module, function,
  arity, native_fn, capability) -> Result<NativeEntry, NativeReplacementError>`
  — atomic replacement of an already-occupied MFA returning the previous
  `NativeEntry` whole (function + capability, directly delegatable). A
  replacement, not an upsert: a vacant MFA returns typed
  `NativeReplacementError::MissingMfa` and leaves the registry observably
  unchanged. The occupied-decision and the swap share one map-entry write
  guard (no lookup-then-replace gap; linearizes at the entry insert), a racing
  lookup observes the whole previous or the whole replacement entry, never
  torn, and no error path can vacate the slot. Intended for registry
  construction (install the complete BIF table, then fence selected MFAs
  before starting scheduler workers) — the driver is aion's spawn-reservation
  fencing of `erlang:spawn/1` / `erlang:spawn_link/1` over the complete Gate-3
  table. Scoped to normal-scheduled BIFs: the replacement entry is written
  with `dirty_kind: None` (the returned previous entry keeps its `dirty_kind`
  intact). Gate tables, scheduler, wasm, and exit surfaces untouched
  (insertion-only change).

## 0.15.3 — 2026-07-18

### Added

- Additive Scheduler exit-observation API: `take_exit_outcome(pid)` — a
  non-blocking, consuming, exactly-once take of a process's `(ExitReason,
  OwnedTerm)` outcome, backed by a durable per-process finalization token that
  survives both legacy tombstone FIFO eviction and the take itself (permanent
  residue pinned at 40 bytes per finalized process, test-asserted) — and a
  single-subscriber bounded exit-event stream (1,024 events) whose `Exited`
  notification is published only after the outcome is takeable, with typed
  `Lagged` overflow and scan-based recovery. Existing exit surfaces
  (`run_until_exit`, `peek_exit_reason`, diagnostic takes, `terminate_process`)
  are semantically unchanged.
- `ExecError::GuardBifUnavailable` with a typed four-arm `GuardBifResolution`,
  exact-string diagnostics through `format_with_atoms`, and an
  `unresolved` import report on `HotLoadResult` (which consequently no longer
  derives `Copy`) — the EMB-001/EMB-002 pair.

### Changed

- The `LitT` chunk emitted by the `.beam` encoder (`encode` feature) changed
  to the tear-ruled **candidate C zero-prefix uncompressed form** (ENC-001): a
  zero u32 size prefix followed by the raw literal-table bytes, replacing the
  zlib-compressed body produced through 0.15.2. The emitted bytes are now a
  pure function of the literal table — no compressor, no ambient
  `Compression::default()`, no dependency-resolved variance — and are
  byte-symmetric with the form the erlc/gleam toolchain emits across the
  committed fixture corpus. Consequences: emitted `.beam` bytes differ from
  0.15.2's output for any module carrying literals, so downstream content
  hashes over emitted bytes (e.g. aion package version identity) shift ONCE on
  the next recompile — expected and correct, never a regression. The decode
  side is unchanged and loads both forms: legacy compressed `LitT` chunks keep
  loading forever.

## 0.15.2 — 2026-07-15

### Fixed

- `make_fun` and `put_map` now route near-full-heap allocations through the
  GC's `ensure_space` safety net (collect, then grow) instead of calling the
  raw heap allocator, which surfaced `heap full: requested N words with M
  available` as a fatal VM execution error. Hit in production by aion's first
  direct-BEAM AWL child workflow: any process whose nursery is near-full at a
  `make_fun` died ~150ms after spawn. Both reservations run before the
  instruction's terms are copied into Rust locals, so a safety-net collection
  cannot leave the closure's free variables or the source map dangling.
- DOWN-message heap reservations under-counted by one word (7 reserved for
  the 8-word local message, 11 for the 12-word remote one). A watcher heap
  with exactly the reserved word count free failed the final tuple allocation
  and the watcher was killed instead of receiving `{'DOWN', Ref, process,
  Pid, Reason}`.

## 0.15.1 — 2026-07-13

### Fixed

- `SupervisionFacility::monitor` against an already-tombstoned target now
  delivers the immediate DOWN through the same dual-slot admission path as
  normal exits: a `Present` watcher is enqueued and woken after the message is
  visible, and an `Executing` watcher receives it via `pending_down_messages`
  merged at store-back. Previously the DOWN was silently dropped for any
  watcher not in the `Present` slot — including every native host observer
  registering while executing — while the result still claimed
  `immediate_down: true`.
- `MonitorResult::immediate_down` is now truthful: it reports whether the DOWN
  was actually admitted, and is `false` when the watcher slot is absent or the
  watcher has already exited.

### Added

- `Scheduler::monitor_with_result(watcher_pid, target_pid)` returns the full
  `MonitorResult` so embedders can observe the immediate-DOWN case. The
  existing `Scheduler::monitor` keeps its signature and delegates to it.

## 0.15.0 — 2026-07-13

### Added

- `Scheduler::send_to_mailbox(pid, OwnedTerm)` is the public threaded-runtime
  host-to-process message primitive. It deep-copies arbitrary owned terms into
  the receiver heap, preserves FIFO with existing atom/timer deliveries, and
  wakes a waiting receiver only after the message is visible. Delivery racing
  an executing slice is merged at store-back and observed by the receiver's next
  receive without a lost-wake window.
- `MailboxSendError` replaces boolean ambiguity for the new API with typed
  `NoSuchProcess`, `ProcessTerminated`, `ProcessSlotUnavailable`,
  `HeapAllocationFailed`, and `InvalidMessage` failures.

## 0.14.0 — 2026-07-12

**The artifact of record for the embedder-composition campaign** (composition
commits 1–6: `docs/EMBEDDER-COMPOSITION-SPEC.md`, `docs/READINESS-CONTRACT-SPEC.md`,
`docs/READINESS-REGISTRATION-API.md`). This version contains exactly the tree
0.13.2 shipped; the minor bump exists because the campaign's public-API breaks
below cannot honestly ride a patch version — which is also why **0.13.1 and
0.13.2 are yanked** (see their entries).

### Breaking

- `Scheduler::dirty_cpu_pool` / `dirty_io_pool` are replaced by
  `try_dirty_cpu_pool` / `try_dirty_io_pool` returning `Option<&DirtyPool>` —
  a composed scheduler can genuinely have no pool, and the old signatures
  could not represent absence without panicking. `#[doc(alias)]`es preserve
  discoverability under the old names.
- `Scheduler::distribution_connections` / `distribution_config` are replaced
  by `try_distribution_connections` / `try_distribution_config` (same reason:
  `distribution: None` now builds NOTHING — honest absence).
- `ExecError::ServiceUnavailable { service }` — new variant, breaks exhaustive
  matches. Raised when a native dirty call reaches a `Disabled` dirty pool;
  the BEAM-visible exit reason stays the plain `error` it always was.
- `SpawnError::SchedulerTearingDown` — new variant, breaks exhaustive matches.
  Every spawn facility entry point refuses (rather than mutating) once the
  scheduler's teardown drain has closed admission.
- `dirty_cpu_threads: Some(0)` / `dirty_io_threads: Some(0)` now mean
  **Disabled** (zero threads, typed refusal at the pre-suspension gate);
  previously zero rounded up to one thread. `None` keeps the eager legacy
  defaults for one release (see the `Scheduler::new` migration note below).
- Ancillary-service thread names are now service-distinct (each ring names
  its own workers; `DEFAULT_RING_THREAD_PREFIX` is the exported default, and
  `create_ring_with_prefix` / `try_create_ring_with_prefix` exist for
  embedder-named rings). Anything keying on the old shared thread names must
  update.
- New **default feature `readiness`** (requires `threads`; adds the
  already-in-lock `mio` as a direct dependency). Default-features consumers
  compile the readiness registration service; the SERVICE stays composed-off
  unless selected (`FromConfig` ⇒ `Disabled`) — feature-compiled and
  service-enabled are deliberately different defaults (registration-API doc
  §8 OQ-1). `default-features = false` consumers (wasm/cooperative) are
  untouched.

### Added (campaign surface beyond the composition entrypoint)

- Per-service `ServiceMode` model with a stable identity per instance, and
  `Scheduler::service_inventory()` — every ancillary service reports mode,
  configured-vs-actual thread counts, thread names, and fd classes;
  transient-thread classes report as policy lines. The §5 permanent
  assertions (inventory ≡ OS probe, signed 5 ms idle floor with its
  `IDLE_PARK_TIMEOUT` / `IDLE_WAKES_PER_SEC_PER_WORKER` linkage) pin it.
- **Readiness registration service** (composition commit 6, contract §3
  shape (b)): register an fd + durable atom marker for a pid, get woken by
  marker enqueue on readiness — the enif_select-class primitive that lets an
  idle consumer park instead of poll. One poll thread per service instance;
  `Owned` per scheduler or ONE `SharedReadiness` injected across many
  (delivery routes home by registration identity; generation-minted
  `ReadinessToken`s make fd reuse safe). In-slice surface:
  `ProcessContext::readiness_facility()` / `NativeContext::readiness_facility()`
  (register + rearm); host-side acknowledged `Scheduler::readiness_deregister`.
  Poll-thread death degrades to typed `ReadinessError::ServiceFailed`
  refusals, honest `actual: 0` inventory, and bounded deregistration.
- Teardown-admission gating across mutating facilities: dirty submissions,
  every spawn path, message/link delivery, ETS create/delete/transfer,
  group-leader set, supervision exit signals, timer arm/cancel, and readiness
  register/rearm all hold an admission the shutdown drain waits out — a
  mutation cannot land after teardown returns, and post-drain calls refuse
  typed (`ReadinessError::TeardownInProgress` for readiness; existing typed
  surfaces elsewhere).
- `ConnectionManager::disconnect_all()`; per-connection teardown that
  actually closes: an atomically-CLOEXEC teardown dup + `shutdown(2)` makes
  connection closure independent of a wedged writer mutex (budget: one extra
  fd per live connection, ledger-signed).

### Added

- `Scheduler::with_services(config, services, module_registry)` and
  `with_services_and_code_server(..)` — the additive composition entrypoint
  (spec §2.2). `SchedulerServices` describes each ancillary service (dirty CPU/
  IO pools, file/standard/generic IO rings, distribution bundle) with a
  per-service choice; an explicit choice WINS over the matching legacy
  `SchedulerConfig` knob, a `FromConfig` choice defers to it. Non-service knobs
  (`thread_count`, node identity, queue depth, telemetry, private data) always
  apply. `SchedulerConfig`'s existing fields and exhaustive-literal shape are
  unchanged — this is purely additive.
- Named profiles: `SchedulerServices::full_runtime()` (today's full standalone
  VM — every service `FromConfig` plus distribution turned on with a default
  config), `SchedulerServices::minimal()` (every ancillary service `Disabled`:
  no dirty pools, no ring, no process 0, no distribution — only the requested
  normal workers run), and `SchedulerServices::from_config()` (the legacy
  profile that `Scheduler::new` maps to).
- Shared dirty pools: inject an embedder-owned `Arc<DirtyPool>` into several
  schedulers with `SchedulerServices::shared_dirty_cpu` / `shared_dirty_io`.
  The pool is used by each scheduler but joined by NONE of them (the embedder
  owns teardown). Safe now because dirty completion routes by the oneshot the
  submission carries, not by any per-scheduler table.
- `SharedIoRing` + `WithServicesError`: the injectable shared-IO-ring handle and
  the typed refusal `with_services` returns for it. Shared IO rings are refused
  this release — cross-scheduler completion routing lands with the §3.9 routing
  gate in a later commit — so the composition surface is complete and the
  refusal is loud and by name rather than a silent misroute.

### Changed

- The `beamr-cli` runner opts into `SchedulerServices::full_runtime()` instead
  of setting `distribution: Some(default)` on the raw config directly. No
  user-visible behavior change.

### Behavior changes (migration)

- **`Scheduler::new` is the legacy profile for one release.** It preserves
  today's EAGER per-knob defaults (a `num_cpus`-sized dirty CPU pool, a
  10-thread dirty IO pool, a live file-IO ring, a live standard-IO ring with
  process 0) as a migration bridge. Embedders that want a specific service
  footprint should move to `Scheduler::with_services` with `minimal()` /
  `full_runtime()`. Distribution already follows `config.distribution` honestly
  (`None` builds neither runtime, since 0.13.0).
- **Replay disables distribution entirely.** Under replay a
  `distribution: Some(config)` bundle is now `Disabled` — NEITHER the outbound
  sender nor the net-kernel runtime is built (previously the net-kernel runtime
  was still constructed, whose live `connect_node` dial performed real network
  IO behind a disabled facade during replay). No replay path reads live
  distribution state; every distribution BIF already resolves to absence
  (`noconnection` / `false` / `[]`). The one observable flip: `is_alive/0`
  reports `false` under replay (spec-§3.6-consistent for a node with no
  distribution service).
- **Distribution is one bundle behind one manager.** The outbound sender and
  the net-kernel now share a single heartbeat-enabled `ConnectionManager`
  (previously two disjoint connection tables). Direct remote sends are
  bounded: a wedged peer writer yields `NoConnection` + connection retirement
  at the drain's 5 s write timeout instead of hanging; `connect_node` carries
  a 15 s whole-attempt deadline; `is_alive/0` is `true` only with a live
  distribution service AND a non-default node name. Teardown joins the
  runtime workers to completion (no leaked runtime threads), safe from any
  calling context.
- Process 0 (group-leader IO server) is registered exactly when the
  standard-IO ring is `Owned`; under `minimal()` there is no process 0, and
  top-level group leaders seed from a dead-leader sentinel rather than
  self-queueing IO forever.

### Fixed

- A kill landing in the store→register park gap left a dead pid's wait-set
  entry behind forever, and a link-cascade kill of a stored process stranded
  its body, owned fds, pg memberships, and metric state while silently
  dropping its remote-link EXITs. Process finalization is now exactly-once
  behind two ownership tokens (table token for pid-keyed work, body token for
  resource release), and the cascade path finalizes like a direct kill.
- The readiness deregistration epoch handshake had a lost-wakeup window (a
  notify could land between a waiter's predicate check and its wait, with no
  second chance on a tickless poller); every predicate-state writer now
  passes through the epoch lock before notifying.
- The readiness in-slice surface was reachable from BIF-path
  `ProcessContext` but not from native-handler `NativeContext` — caught by
  the first external consumer, fixed the same night, and the
  first-external-consumer gate (an integration test consuming public paths
  only) is now standing verification doctrine.

## 0.13.2 — 2026-07-12 [YANKED]

0.14.0's tree, released under a patch version: 0.13.1 plus the readiness
§1.4 conformance fix (`NativeContext::readiness_facility`). **Yanked
2026-07-12** because the composition campaign's public-API breaks (see
0.14.0 § Breaking) cannot ride a patch bump. Use 0.14.0 — it is the same
tree with an honest version.

## 0.13.1 — 2026-07-12 [YANKED]

First release of the embedder-composition campaign (composition commits 1–6,
including the readiness registration service). **Yanked 2026-07-12**, same
reason as 0.13.2: breaking API changes under a patch version. Use 0.14.0.

## 0.13.0

Distribution grows real cross-node supervision: LINK/UNLINK/EXIT/EXIT2 now
travel the wire (previously only SEND/REG_SEND/PG_UPDATE did — cross-node
links compiled but never delivered a death signal), backed by a
multi-subscriber connection-event hub and a generation-pinned must-deliver
control lane. Specs: `docs/CONN-EVENTS-HOOK-SPEC.md` and
`docs/DIST-CONTROL-WIRE-SPEC.md` (each carries an as-built addendum recording
where the landed code deviates); decision record: ADR-012.

### Added

- Connection-event hub (`docs/CONN-EVENTS-HOOK-SPEC.md`):
  `ConnectionManager::subscribe_connection_events` /
  `subscribe_connection_events_with_snapshot` / `unsubscribe_connection_events`
  deliver generation-tagged `NodeUp`/`NodeDown` events to any number of
  subscribers with per-node alternation and exactly-once-per-session
  guarantees. `NodeUp` carries `peer_creation` so subscribers can distinguish a
  peer VM restart (all remote pids dead) from a connection blip (pids
  survive). The snapshot variant synthesizes catch-up `NodeUp`s for
  already-live sessions under a stitch-race-free gate — subscribing late
  misses nothing and double-sees nothing. Dispatch is synchronous on the
  transition thread (events are facts by the time `register_connection` /
  `connection_down` returns) with owner-thread reentrancy; the pre-existing
  single replace-on-register hook slot is now a compatibility facade over the
  hub (registered last, byte-stable semantics for 0.11-era embedders).
- A peer VM restart that re-dials before the old socket dies (live
  displacement or canonical-arm bounce with a changed `creation`) now closes
  the old session properly: `NodeDown(old)` + `NodeUp(new)` both fire, pg
  groups are purged, and `noconnection` reaches linked processes — previously
  the redial coalesced silently into the stale session.
- Cross-node link supervision on the wire (`docs/DIST-CONTROL-WIRE-SPEC.md`):
  OTP control opcodes LINK=1, EXIT=3, UNLINK=4, EXIT2=8 encode/decode and
  deliver. `Scheduler::link_remote` / `unlink_remote` establish and sever
  links whose EXIT signals actually cross the wire; exit reasons map per OTP
  semantics (`kill` crosses as `killed` on link-EXIT, raw on EXIT2), trapping
  targets receive `{'EXIT', From, Reason}` with a correctly-built external-pid
  source, and delivery contracts DC-1..DC-6 pin exactly-once semantics: for
  every established link, a dying peer process yields exactly one of {wire
  EXIT, `noconnection` backstop} — never zero, never two.
- Must-deliver control lane: link controls ride a dedicated 256-slot
  generation-pinned queue with a biased drain (controls before data). The lane
  cannot silently drop: overflow marks the pinned connection down
  (`ConnectionDownReason::ControlOverflow`, new variant) so the `noconnection`
  backstop delivers what the wire could not. This replaces the data path's
  silent-drop-at-1024 behavior for supervision traffic.
- `ExitReason::NoProc`; `RemotePid` link endpoints normalize `serial` to 0 at
  the facility boundary (documented on `link_remote`) so an
  embedder-constructed nonzero serial cannot dodge the EXIT-delivery equality
  gate.
- Telemetry counter `beamr.distribution.control_frames_dropped` (reason
  attribute) for malformed/misaddressed inbound control frames; heartbeat
  keepalives are excluded.

### Fixed

- Remote-link removals recorded while the target process was mid-slice
  (Executing) were silently resurrected at store-back — the checkout merge was
  add-only. Consequences before the fix: `unlink/1` on a remote pid was a
  deterministic local no-op, and the exactly-once EXIT gate could double-fire
  (a second spurious `noconnection` after a real wire EXIT, killing a
  non-trapping process that had survived a `normal` exit). Store-back now
  reconciles removals with metadata authoritative, mirroring the monitors
  merge.
- Remote EXIT delivery to a trapping process built the external source pid,
  then crossed a GC-capable allocation without rooting it — under nursery
  pressure the delivered `{'EXIT', From, Reason}` tuple held a dangling `From`.
  The pid and tuple are now one contiguous allocation behind one reservation.
- An inbound LINK racing a write-side connection-down (write timeout, control
  overflow, `disconnect_node`) could establish a link the backstop scan had
  already passed — the death signal was lost forever. The apply now rechecks
  the origin connection post-establish and delivers the missed `noconnection`
  (the exactly-once gate keeps both race orders single-delivery).
- Local pids beyond the wire's u32 range are refused at `link_remote`
  (`RemoteLinkError::BadTarget`) instead of tearing the whole connection down
  on every outbound control after the pid counter passes 2^32.

### Removed

- The orphaned test-only control planes `distribution/control_lifecycle.rs`
  and `distribution/control_monitor.rs` (never wired to the wire; their
  numeric opcode-table test moved to `control_link.rs`) and the scheduler's
  `ControlRouter` (accumulated EXITs into a never-drained queue). The landed
  wire path replaces all three.
- `DistributionFlags::offered()` no longer advertises `ATOM_CACHE` — the codec
  never implemented cache references, so offering it invited undecodable
  frames from spec-conforming peers. Accepting it from peers is unchanged.

### Known limitations (deliberate, recorded)

- Remote monitors stay at local-only semantics this release: `BEAMR_MONITOR`
  opcode 102 is reserved in the codec and rejected on the wire; the monitor
  stage needs external-pid plumbing at the BIF layer first (spec §1.3).
- Links are node-keyed, not generation-keyed: a link established in the
  narrow window while a `NodeDown(g)` is dispatching after a redial installed
  session g+1 can be spuriously severed. The fix needs per-link session
  pinning through public API shapes — deferred for a design ruling rather
  than rushed (DIST-CONTROL-WIRE-SPEC as-built addendum, finding W2).
- The kill-9 verification harness (true SIGKILL of a subprocess peer, as
  opposed to in-process socket-drop e2e — which this release does test) is
  deferred to a follow-up work item.
- `cargo check -p beamr --no-default-features` is red with 1020 pre-existing
  no-std errors, byte-count-identical from baseline ec5d7f8 through this
  release — the series introduced no new breakage, but that gate leg is
  waived, not green. Restoring no-std is a separate work item.

## 0.12.1

### Fixed

- Run-queue priority lanes pop FIFO from the owner side instead of LIFO. A permanently-runnable native process (one that returns `NativeOutcome::Continue` every slice — a busy-poll connection loop, for example) was re-popped immediately after its own requeue, forever: every other pid on that scheduler thread starved indefinitely, work stealing could not rescue them (the owner's queue never exposed more than one item outside a nanoseconds-wide window), and messages delivered to a starved pid sat in its mailbox unobserved while `wake_process` correctly no-opped (the pid was runnable the whole time, not waiting). With N scheduler threads and more than N spinning natives, exactly N processes made progress. Both published crates.io releases 0.11.0 and 0.12.0 ship the LIFO lanes — consumers running busy-poll natives on a shared scheduler should upgrade. Regression-pinned under the real supervised spawn path with spawn/exit churn.

## 0.12.0

### Added

- `Scheduler::spawn_link_closure(parent_pid, closure_term)`: spawn a linked child process that runs a zero-arity closure (thunk). Unlike the `args: Vec<Term>` spawn entrypoints — whose argument terms are NOT heap-copied and require the caller to keep any backing heap alive — the closure's environment (free variables) is deep-copied into the child's own heap via the mailbox copy machinery before the child becomes runnable, so the caller's heap may be collected, mutated, or freed the moment the call returns. The child heap doubles on `HeapFull` up to a 2^26-word cap. Target resolution matches `call_fun` (generation match with unique-id validation, unique-id fallback across generations, old-generation fallback); export funs (`fun m:f/0`) resolve through the export table; native-entry funs are not spawnable. The link is established atomically at spawn (no unlinked window) and the child does not trap exits. Built for Aion's in-VM activity tier (linked activity child processes running SDK-supplied thunks).

## 0.11.0 — 2026-06-28

The cooperative wasm runtime release (WR-0..WR-10, this range landing
WR-2..WR-10): beamr's native-process model runs on the single-threaded
cooperative `WasmScheduler` — no tokio, no crossbeam channels, no OS threads
in the execution path. `beamr-wasm` 0.5.0 rides along. The public threaded
API is unchanged (additive + cfg-widening only).

### Added

- Native processes dispatch through the unified cooperative `run_until_idle`:
  native and bytecode processes share a single host pump, with native slice
  outcomes folded into the same `WasmRunSummary` and yielded-requeue buffer
  the bytecode arm uses (WR-3).
- Cooperative native timers: `WasmScheduler` carries a shared `TimerWheel`,
  so `NativeContext::send_after`/`schedule` build real `Deliver` timers
  instead of hitting an inert `None` wheel; expirations drain once per turn
  via `tick_native_timers`/`tick_native_timers_at` (WR-4).
- Cooperative supervision and restart: `spawn_native`'s `link_to` establishes
  the bidirectional link, exit propagation delivers `{'EXIT', From, Reason}`
  to trapping links and applies `should_die_from_signal` semantics to
  non-trapping ones (the predicate is now shared with the threaded path by
  construction), and restart is the trapping supervisor re-invoking the
  retained factory (WR-5). A review pass rewrote the link cascade as a
  transitive worklist mirroring the threaded path — the initial in-place kill
  let grandchildren survive and left dead processes re-enterable as zombies.
- `spawn_actor_cooperative` + `CoopActorRef`/`CoopSenderHandle`:
  fire-and-forget `cast` and non-blocking `call_async`/`call_async_timeout`
  returning a host-pumpable `CallFuture<Reply>`; ref correlation reuses the
  threaded envelope machinery, so concurrent calls never cross replies (WR-6).
- Native handlers reach the wasm async-NIF seam: `NativeContext::start_async`
  parks the handler without blocking the event loop, and `complete_async`
  delivers the completion as an `{ok, Value}`/`{error, Reason}` mailbox
  message on a later turn (WR-7).
- `DynActor`/`ReplyFn`/`WireTerm` — a term-carrying actor an untyped host
  drives over `call_async` with no new wire code — plus
  `NativeContext::alloc_owned_term`; on the beamr-wasm JS seam,
  `WasmVm::spawn_actor(handler)`, `call(pid, request)` returning a real
  Promise, and `cast` (WR-8).
- Wasm time base: the native timer wheel, the cooperative timer seam, and
  the in-memory replay driver read `web_time::Instant` (performance.now() on
  wasm; identical to `std::time::Instant` on native). beamr-wasm gains the
  requestAnimationFrame host pump: `WasmVm::pump_once`, `start_pump()`
  returning a `PumpHandle`, and idempotent `PumpHandle::stop()` (WR-10).
- Distribution reconnection hardening (HS-4/HS-5): `connect_node` treats a
  down-but-not-yet-reaped connection entry as not-connected, so re-dial after
  a dropped link is deterministic instead of being told the peer is up; plus
  a 3-node full-mesh handshake-convergence integration test (six
  simultaneous dials, per-node runtimes, hard watchdog) pinning the 0.10.0
  deadlock fix in CI.

### Fixed

- Safety-net GC collections inside `put_list`/`put_tuple2`/`update_record`
  conservatively root the full X register file. The hardcoded `live_x` of
  256 both under-rooted and NIL-cleared any live term in a register at index
  ≥ 256 — silent corruption. These opcodes carry no Live operand in this
  VM's bytecode, so conservative full-width rooting is the only sound choice
  (#106).

## 0.10.0 — 2026-06-27

### Fixed

- Distribution handshake deadlock (HS-0..HS-3,
  `docs/DISTRIBUTION-HANDSHAKE-DESIGN.md`): simultaneous cross-dials could
  hang `connect` forever and prevent a ≥3-node mesh from forming. Three
  coordinated fixes: whole-handshake deadlines (default 5 s,
  `with_handshake_timeout`, new `HandshakeError::Timeout`) so the outbound
  connect always returns and no accept-side responder parks forever (HS-1);
  race-safe connection install — `register_connection` dedups against an
  existing live link per peer name, dropping the newcomer's stream and
  replacing stale down entries, so two simultaneous handshakes cannot leave
  a clobbered, orphaned reader (HS-2); and the OTP simultaneous-connect
  tie-break (`ok`/`ok_simultaneous`/`nok` status bytes decided by node-name
  comparison) so exactly one symmetric link survives per pair — the losing
  initiator's `nok` folds into a benign non-retrying success (HS-3). The
  pre-fix silent-peer hang and simultaneous-dial mesh scenarios are pinned
  as regression oracles (HS-0).

### Added

- Wasm-runtime port groundwork (WR-0/WR-1,
  `docs/WASM-RUNTIME-PORT-DESIGN.md`): a new `cooperative` Cargo feature
  (std + crossbeam-queue only) with cooperative spawn/local-send facilities
  and a native-aware turn on `WasmScheduler` proving a native Actor runs
  cooperatively; host-only modules (io/jit/timer/replay/distribution/hook)
  are feature-gated so beamr compiles toward `wasm32-unknown-unknown` with
  no default features. The native default build is unchanged and the
  cooperative build is warning-free.

## 0.9.0 — 2026-06-24

The distribution layer's minor-release marker: promotes the cross-node work
landed in 0.8.3 (OTP handshake, async sender, cross-node pg) to a minor
version.

- Added `Scheduler::atom_table()` so distribution-facing embedders intern
  names into the SAME atom table the scheduler uses internally — pg
  group/scope atoms and the node atoms from
  `ConnectionManager::connected_nodes()` are indices into it, so a
  separately-constructed table would not match. Mirrors the accessor
  `WasmScheduler::atom_table()` already exposes.

## 0.8.3 — 2026-06-24

The distribution layer lands: cross-node process groups over authenticated
connections with non-blocking propagation.

### Added

- OTP handshake wired into `ConnectionManager` connect/accept:
  cookie/challenge/MD5-digest auth with constant-time compare and
  cryptographically random challenges; connection identity comes from the
  authenticated `HandshakeResult::remote_name` (the address→atom identity
  seam is deleted); cookie configured via `DistributionConfig`; public
  `Scheduler::start_distribution_listener`. The handshake completes before
  the data-frame read loop starts.
- Distributed process groups: local pg join/leave propagate to every
  connected node via a `PG_UPDATE` control frame (op 101, member as an
  external pid carrying the local node name); inbound frames apply on the
  peer's `PgRegistry`; a connection-down hook purges the lost node's
  members wholesale.
- Async distribution sender (`DistSender`): all outbound distribution I/O
  moves to a single owned 1-worker runtime with a bounded queue. pg
  broadcast enqueues instead of `block_on` on a scheduler worker thread
  (killing the latency cliff), process exit purges pg membership locally and
  propagates the leave async (never blocking the death path), and writes
  carry a 5 s timeout so a wedged-but-connected peer is marked down instead
  of stalling propagation cluster-wide.

### Fixed

- The distribution control-frame handler captured a strong `SharedState`
  reference, so schedulers with distribution enabled never dropped; it now
  upgrades a `Weak` per frame (regression-pinned: `strong_count == 0` on
  drop).

## 0.8.2 — 2026-06-24

- Timer messages are actually delivered: the timer wheel was
  receive-timeout-only, so `send_after`/`start_timer` scheduled messages
  that never reached any mailbox. Timers now carry a `TimerKind`
  (`ReceiveTimeout` keeps the mark-and-wake code-jump path; `Deliver` pushes
  the message into the target mailbox with Executing-slot-safe semantics and
  wakes the process).
- Native processes gain timer access: `NativeContext` carries an optional
  shared timer wheel with `schedule`/`send_after`/`cancel_timer`.
- Replay log `FORMAT_VERSION` 1 → 2: the timer-kind byte round-trips, and an
  unknown byte is `InvalidFormat` rather than a silent default.

## 0.8.1 — 2026-06-23

- Corrected the `recv_marker` opcode family to OTP numbering
  (173=bind/2, 174=clear/1, 175=reserve/1 — beamr had the three rotated with
  mismatched arities), which desynced decoding through the receive prologue
  and made the loader reject valid modules with "export label N does not
  exist"; `recv_marker_bind`'s second operand is modelled as a register
  (Ref), not a label.
- Added `Scheduler::peek_exit_reason` — a non-blocking, non-consuming read
  of a dead process's exit reason, for supervisors that must observe an
  external kill without parking on `run_until_exit`.
- Exit tombstones are bounded: the unbounded pid→ExitReason map is now an
  insertion-ordered store with FIFO eviction above 65,536 live entries,
  evicting a pid's paired exit-result satellites together with its
  tombstone — closing a slow per-connection/per-request leak with read
  semantics unchanged (eviction can never strand a blocked
  `run_until_exit`: the awaited tombstone is always the newest entry).

## 0.8.0 — 2026-06-23

The native-process release: Rust code participates in the process model as
real processes.

### Added

- Native-process core (NATIVE-001): a native process IS a `Process` carrying
  a Rust `NativeHandler` — factory-based `spawn_native`, `run_native_slice`,
  and `NativeContext::send` through the real `LocalSendFacility`. Reuses the
  park-gap protocol, exit tombstones, and pending-message merge verbatim; no
  new process-slot variants or sync primitives.
- Native-process supervision (NATIVE-002): links, monitors, exit signals,
  trap_exit, and factory-based restart reuse the pid-keyed exit-propagation
  machinery unchanged; adds `NativeContext::set_trap_exit` and generic
  `Scheduler::is_native`/`monitor`/`exit_signal`.
- Ergonomic actor API (NATIVE-003): gen_server-style `Actor` trait
  (`handle_call`/`handle_cast`), ref-correlated `call` and fire-and-forget
  `cast`, and public `spawn_actor` returning a Clone-able `SenderHandle`.
  Blocking `call` lives only on the external `SenderHandle`; handlers get a
  cast-only `ActorContext`, so the call-deadlock is unreachable by
  construction. Feature-gated; the bytecode path is untouched.

### Fixed

- A trapping process that was mid-slice (Executing) when a linked process
  exited normally never received `{'EXIT', Pid, normal}` — both the
  `process_exit_signal` and `exit_signal` (erlang:exit/2) Executing arms
  gated delivery on a non-normal reason. Both now gate on trap_exit alone,
  matching the Present arm, the remote sibling, and OTP semantics.

## 0.7.0 — 2026-06-22

### Fixed

- Cross-process local send actually delivers: `B ! Msg` between two
  processes driven through the real Send opcode silently dropped
  (`messaging::send` only delivered to an in-hand receiver, and the
  scheduler always passed none). A new `LocalSendFacility` delivers via the
  I/O-delivery template — a Present receiver gets a deep copy onto its heap
  with push-before-wake, an Executing receiver (mid-slice on another thread)
  gets the message ETF-encoded and decoded onto its heap at store-back, and
  self-sends deliver to the in-hand process. Replay clock observation
  happens under the slot lock.
- ETF decode gained reference arms (`NEWER_REFERENCE_EXT` 90,
  `REFERENCE_EXT` 114): the encoder emitted `NEWER_REFERENCE_EXT` but the
  decoder had no arm, so ref-bearing messages (gen_server call tags, monitor
  DOWNs) were silently dropped on the Executing path.

### Added

- Encode/copy failures on the send path surface via a `messages_dropped`
  telemetry counter instead of vanishing.

### Compatibility

- `NativeServices` gains a `local_send` field and is now
  `#[non_exhaustive]`; embedders constructing it as a struct literal must
  update.

## 0.6.4 — 2026-06-16

- Added `erlang:integer_to_list/2` (radix 2–36, OTP semantics).
  gleam_json's error-path hex formatter calls `integer_to_list(I, 16)`,
  which was undefined — crashing workflows that hit JSON parse errors during
  diagnostics rendering.

## 0.6.3 — 2026-06-15

- io_uring backend: added the missing `SendMsg`/`RecvMsg` match arms (the
  new `IoOp` variants made the Linux build fail on a non-exhaustive match);
  implements async sendmsg/recvmsg via io_uring opcodes with heap-stable
  storage for msghdr, iovec, and address buffers.

## 0.6.2 — 2026-06-15

- Linux build fix for io-uring 0.7.12: `Statx::new` went from 5 args to 3 —
  flags and mask are builder methods and the statxbuf pointer is an opaque
  type.

## 0.6.1 — 2026-06-13

- `put_list` and `put_tuple2` self-ensure heap space before allocating: when
  data-dependent decoding builds more cells than the preceding `test_heap`
  reservation covers, the raw bump allocator returned a fatal `HeapFull`,
  bypassing the GC-and-grow path. Both opcodes now call `ensure_space()`
  before reading operands, matching `update_record`.

## 0.6.0

### Correctness

- Off-heap (ProcBin) and sub-binary terms survive the whole BIF surface: `byte_size`/`bit_size`/`binary_part`/`is_bitstring`/`iolist_size`, `binary_to_term`, `code:load_binary` bytes, file/TCP/UDP byte and filename extraction, and the JSON bridge previously accepted only inline heap binaries (≤ 64 bytes) and raised `badarg` on anything larger — the cause of "binaries over 64 bytes kill a resumed workflow with bad argument". All now go through the representation-agnostic `BinaryRef` accessor. `byte_size`/`bit_size` additionally accept bs match contexts: OTP 26+ compilers emit the gc_bif on the reused match-context register for match tails (`<<_, Rest/binary>> = B, byte_size(Rest)`) instead of materializing the tail sub-binary.
- Message sends copy ProcBin terms by sharing their refcounted off-heap bytes and copy sub-binaries' visible ranges threshold-aware; both previously failed delivery with `InvalidBoxedTerm`.
- Published host suspension results (`Scheduler::wake_with_result`/`wake_with_result_for` and the IO-bridge completion seam) are deep-copied into owned storage at publish time and materialized on the owning process heap at slice-start apply — a boxed result term no longer points into publisher storage of foreign lifetime across the publish-to-apply window. Heap space is collected/grown before the apply copy on both the host and dirty completion paths, so arbitrarily large results cannot die on `HeapFull`.
- `call_ext_last` native tail calls are suspension-safe: the y-frame pop is deferred until a clean (non-dirty) native call completes, so a suspending native's wake re-execution no longer double-pops the stack — previously the eventual return landed at the caller's own call site with the result in x0, crashing with `bad function term {ok, ...}` whenever the suspending call's argument expression contained a cross-module call (`fn() { ffi.sleep(duration.to_milliseconds(d)) }`). Code targets and dirty natives keep the eager pop.
- Host results applied at tail-call parks (`call_ext_only`/`call_ext_last`) return to the caller — popping the deferred frame first — instead of advancing past the function's last instruction; the suspension record carries the park's resume continuation, chosen at suspend time. Scope: threaded scheduler — the WASM scheduler's completion apply still advances blindly (known follow-up, consistent with its pid-keyed completion map).

### Compatibility

- `SuspensionRecord` gained a `continuation` field and `interpreter::opcodes::trampoline::handle_suspend` takes the parked call's completion shape; embedders constructing these VM-internal types directly must update. The embedder-facing `Scheduler`/`ProcessContext` APIs are unchanged.

## 0.5.0

### Correctness

- Suspension protocol redesign (call-identity gating): every result-gated suspension — host await, dirty native call, hook suspend — now carries a per-process monotonically increasing call id recorded at suspend time. Completions are published keyed by `(pid, call id)` and applied at slice start only when the id matches the process's current suspension at its recorded park position; stale completions are dropped instead of being applied blind (the pid-keyed, position-blind application could advance the instruction pointer at the wrong park position — or twice — desyncing execution into "invalid operand for instruction pointer"). Gated host awaits (`ProcessContext::request_await_suspend`, file/UDP/TCP/inet ring operations, `submit_io_and_suspend`) have a wake guard: plain message arrivals can no longer re-execute the await native and double-submit its host work. `request_suspend` keeps its message-wakeable re-execution semantics for re-entrant natives (select, marker awaits) and now returns the suspension call id; `Scheduler::wake_with_result_for(pid, call_id, term)` is the exact completion API and `wake_with_result`/`wake_with_dirty_result` resolve the id at publish time (and return `bool`). `Scheduler::resume_process` is identity-gated (it can no longer resume an in-flight dirty call) and sticky (a resume racing the hook suspension's park gap is recorded and consumed, never lost). Completion application owns the timed-await lifecycle, so a completion-vs-timeout race can neither re-run the native nor leave stale timeout metadata that a later wait would re-arm. Process exit purges all per-pid suspension state. Resuming native continuations may legally re-suspend or trampoline (previously their requests were silently dropped), dirty natives may re-suspend as host awaits or trampoline closures (requests travel through `DirtyResult`), and pending continuations are position-gated so a re-entered await at equal stack depth cannot re-fire a continuation with garbage x0. Scope: threaded scheduler — the WASM scheduler keeps its single-threaded pid-keyed completion map (known follow-up).
- Wave 1 scheduler/VM fixes: opcode 115 (`is_function2`) decodes with its arity operand instead of crashing every literal-arity `is_function/2` guard; `try_case` consumes the current exception so a caught-and-handled exception no longer surfaces as an exit exception; the Wait arm registers in the wait set before its final mailbox recheck (lost-wakeup race against concurrent delivery); a dirty suspension whose resume raced the park is unparked by a fallback recheck.
- Registered `erlang:is_function/1` and `is_function/2` as callable BIFs — body-position calls and variable-arity guards (which compile to the guard-BIF instruction) previously crashed at call time on the unresolved erlang import.
- `receive ... after` timeouts are delivered per BEAM semantics: timer expiry falls through to the `timeout` instruction (the after-body) instead of re-scanning the receive loop and re-arming forever, and the receive timer stays armed across non-matching message wakeups instead of being cancelled with a stale ref that blocked re-arming. Timer expiry is now mark-and-wake: the owning scheduler thread applies the timeout jump at slice start, closing the expiry-vs-park race (the wait-arm recheck also notices a timer that fired inside the park gap). Scope: threaded scheduler only — the WASM scheduler (cancel-on-enqueue) and the JIT wait path (clear-ref on re-execution) still re-arm the full timeout after a non-matching wake; both are known follow-ups.

### Output

- Lists of printable latin1 character codes format as double-quoted strings (`[104,105]` prints as `"hi"`), matching `io_lib:printable_list/1` semantics and the Erlang shell.

## 0.4.9

- `bs_match` `'=:='` chunks compare as integer values, fixing literal-pattern matches against binary segments.

## 0.4.8

- Dirty-parked processes stay parked across mailbox wakes: a message arriving while a dirty native call is in flight no longer schedules a slice that re-executes the call instruction.

## 0.4.7

- Only dirty results resume dirty-call suspensions; mailbox deliveries can no longer resume a process suspended on an in-flight dirty native call.

## 0.4.6

- NIF private data — the `enif_priv_data` equivalent, carried into continuation resume contexts.
- Closed a lost-wakeup race between host delivery and NIF suspend.

## 0.4.5

- Allocation-list fun entries reserve the full closure base, fixing heap reservation for funs allocated through allocation lists.

## 0.4.4

- Release of the 0.4.3 series (no code changes beyond the version bump).

## 0.4.3

- Removed all remaining `gleam_stdlib`/`gleam@` native stub shadows; OTP-level natives made contract-exact. Fixed seven VM bugs found by extended gate stdlib coverage, plus binary-match opcodes and `string:trim` semantics.
- Deterministic replay: causal message ordering, persisted replay logs, a record/replay CLI, and hardened log validation.
- WASM scheduler: receive timers and async NIF promises bridged, direct JS term conversion, JS message send and callbacks, bundle builder with an edge-worker example.
- Workflow telemetry bridged into process tracing; Aion `with_timeout` trampoline continuation variant.

## 0.4.2

- Release bump for the correctness work documented under 0.4.1 below (core correctness, structural GC rooting, fresh Gleam gate).

## 0.4.1

### Correctness

- Fixed `STRING_EXT` literal materialisation: ETF tag 107 is a compact list of byte-sized integers and now becomes cons cells instead of a binary (root cause of `lists:reverse/1` badarg on list literals).
- Exit results and exceptions are captured as owning deep copies before process heap teardown, fixing use-after-free formatting of CLI results and error reasons.
- Native BIF allocation sequences are now structurally GC-safe: self-rooting allocators, `with_rooted`/`rooted_push` scopes, and native continuation state traced as process roots (previously x-registers above the BIF arity were not roots).
- `bs_create_bin` handles real compiler-emitted segment forms; big-integer literals load through the constant pool; unary minus/`abs` and integer-to-string conversions cover bignums.
- Capability-denied imports bind an explicit `ResolvedImportTarget::Denied` variant instead of comparing function pointers, which broke under release codegen.

### Features

- Export funs (`fun M:F/A`): EXPORT_EXT literals materialise as callable values dispatched by MFA through `call_fun`/`call_fun2` and native trampolines — passing `int.to_string` to `list.map` works.
- Native OTP 27 `json` module (`decode/1`, `encode/1`, `encode_integer/1`, `encode_float/1`, `encode_binary/1`), dependency-free and always on, with the OTP error contract `gleam_json` matches on.
- `beamr imports` also lists deferred module dependencies, so empty output now genuinely means the module runs standalone.

### Fixes

- Removed native stubs that shadowed real Gleam stdlib bytecode with wrong semantics (`gleam@list:map` argument order, `gleam@string_tree:split` returning nil).
- The CLI shares its atom table and BIF registry with the scheduler; spawn failures report resolved MFA names instead of `#<unknown atom>`.
- `io_lib_format:fwrite_g/1` keeps a decimal point in whole floats (`1.0`, not `1`).
- Fixed a whole-suite DashMap self-deadlock and a TCP fd-reuse test flake; the test suite (1,500+ tests) and strict clippy (`-D warnings`) gate the workspace.

## 0.4.0

### Headline features

- Added always-on JIT compilation via Cranelift, including runtime profiling, native-code cache support, and adaptive threshold tuning through scheduler configuration.
- Added AOT/native bundle support for exported module functions with Gleam type sidecars. AOT bundles persist a host-target-validated cache envelope and recorded function metadata; native Cranelift function pointers remain process-local and are recompiled on load.
- Added single-binary packaging support with embedded `.beam` archives and runtime loading APIs for packaged modules.
- Added a differential testing framework for comparing beamr behavior with BEAM/Gleam expectations, including JIT-threshold-forced differential runs.
- Added Criterion benchmark targets for JIT comparison and extended JIT comparison workloads.
- Added the new `gleam-types` crate for extracting, serializing, and loading Gleam type sidecars consumed by beamr's typed JIT/AOT paths.

### Breaking changes

- Runtime/API surface now carries JIT state: `SchedulerConfig` includes `jit_threshold`, `SharedState` owns JIT profiler/cache fields, and `Process` tracks JIT runtime/status fields.
- Process/runtime internals gained additional fields for Phase 4 execution state; code constructing these structures directly must use the updated constructors or provide the new fields.

### Release notes

- Publish order is `gleam-types` first, then `beamr` after the `gleam-types = 0.4.0` dependency is available.
- Actual crates.io publishing and pushing `v0.4.0` require explicit project-lead approval.
