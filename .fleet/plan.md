Model: `claude-opus-5` — this seat's own identifier, verbatim. Pin verified
in-band, not assumed: `env | grep ANTHROPIC_MODEL` → `ANTHROPIC_MODEL=claude-opus-5`
(OBSERVED). The harness reports the exact served model ID as
`claude-opus-5[1m]` (the 1M-context variant of the pinned model).

---

# FLIGHT 3 VERIFICATION PASS — read this before §1

Flight `beamracc-lifetimes-flight3`, 2026-08-22, ACP Claude harness.

**This plan was authored by flight 1's planning counsel** (rescued at
`fleet/beamracc-flight1-flagged` = `ba6c7e9f`). Flight 1 died at the BUILDER
seat — its provider refused the request under a cybersecurity policy while
the leg read unsafe-accessor content. That was a **provider refusal, not a
defect in this plan** and not a judgement about this work, which is
Tom-ordered memory-safety repair.

**Per the carry notice this seat VERIFIED the plan against the tree and did
not rewrite it.** §1–§12 below are flight 1's text, preserved. Everything
this seat re-derived is recorded here; sections the tree contradicts carry
an inline `⚠ AMENDED — see V#` marker and are corrected below, never
silently edited.

## Verified UNCHANGED on this tree (re-derived by this seat, not copied)

| Plan claim | Instrument | Result |
|---|---|---|
| §1 sweep — 11 lines, `rg -n "'static" crates/beamr/src/term/` | re-run | **identical listing**, all 11 |
| §1 the five in-family coordinates, `:357` included | `sed` at each line, read by symbol | **all five present at the briefed lines**, no drift |
| §2 Class B (`binary.rs:70`, `binary_ref.rs:30`) | read | confirmed; `parent_bytes` → `Binary::as_bytes` at `accessors.rs:359` confirmed |
| §2 Class C — all 9 coordinates | `sed` at each line | **all 9 exact** |
| §2 Class D — all 3 coordinates | `sed` at each line | **all 3 exact** |
| §2 Class E — `jit/runtime.rs:400`, `ir_exceptions.rs:343`, `atom/table.rs:241`, `io/resource.rs:278` | `sed` | all exact |
| §3 ownership surface — all 9 coordinates | `sed` at each line | **all 9 exact** |
| §3 `ProcessContext<'process>` guard at `context/mod.rs:337` | read | confirmed, incl. `process: Option<&'process mut Process>` |
| §5 public re-export chain (`lib.rs:62` → `term/mod.rs:10-12` → `boxed/mod.rs:12`) | read | confirmed; all four families are public API |
| §5 grep pre-estimate — all 9 rows | re-run | **every count identical** (156/94/111/37/44/23/14/4/2) |
| §6 miri | `cargo +nightly miri --version` after PATH export | **`miri 0.1.0 (8925ea358a 2026-08-20)`** — identical, and the PATH trap is real |
| §6 `compile_fail` doctest precedent at `process/mod.rs:146`,`:155` | read | confirmed, both blocks |
| §7 gates.json — nine legs, names and order | `jq` | confirmed |
| §7 `nostd-ratchet` `CEILING=1072`, strict both directions | read `:81`,`:120`,`:127` | confirmed verbatim |
| §8 `accessors.rs` = 472 lines | `wc -l` | confirmed — 28 from the wall |
| §9 red precedent `gc_rooting_tests.rs` `Process::new(1, 96)` | read `:37` | confirmed |
| §10 branch measurement — 8 divergent refs, counts, merge-base | re-run | **every count identical**; `fix/0163-borrow-across-alloc` 401 behind / 15 ahead, merge-base `67f89c41`, its `term/` diff still **empty** |
| baseline `cargo check --workspace --features beamr/encode` | re-run | 0 errors, `Finished dev profile in 19.29s` |

**The plan's terrain map is accurate.** Six corrections follow; two of them
(V1, V2) are load-bearing and would have burned the builder seat.

---

## V1 — ⛔ §7 AMENDED, LOAD-BEARING: the gate entry does NOT run the legs. It only GRADES them.

OBSERVED — `.fleet/gate-entry.sh` runs exactly one thing:
`./scripts/ci-verdict.sh gate-rc gates.json`.

OBSERVED — `scripts/ci-verdict.sh:34`: `Usage: ci-verdict.sh [GATE_RC_DIR]
[GATES_JSON]`. It is a **grader**: it reads `gate-rc/declared.count` and
`gate-rc/<leg>.rc` / `<leg>.log` files that a **separate canon step** must
have already written. `:47` returns 1 with *"no declared-leg count: the canon
step did not reach its first leg"* when that directory is absent.

OBSERVED — this seat ran `.fleet/gate-entry.sh` on the **untouched** tree.
Its seven embedded self-tests all graded as expected, then:

```
no declared-leg count: the canon step did not reach its first leg
GATE_EXIT=1
```

OBSERVED — the leg-running loop lives in **`.github/workflows/ci.yml:68-112`**,
not in any script under `scripts/`. Its contract, verbatim in shape:

```bash
mkdir -p gate-rc
declared=$(jq -r '.legs | length' gates.json)
echo "$declared" > gate-rc/declared.count
for i in $(seq 0 $((declared - 1))); do
  name=$(jq -r ".legs[$i].name" gates.json)
  cmd=$(jq -r ".legs[$i].cmd" gates.json)
  ( eval "$cmd" ) > "gate-rc/${name}.log" 2>&1   # SUBSHELL + REDIRECT, not pipe
  rc=$?
  echo "$rc" > "gate-rc/${name}.rc"
done
./scripts/ci-verdict.sh gate-rc gates.json
```

Three properties of that loop are load-bearing and `ci.yml:68-93` says so in
its own comments: **no `set -e`** (a failing leg must not abort the loop —
every leg runs, the verdict is taken from the recorded set); **a subshell**
around `eval` (so a leg shaped `cd … && …` cannot silently relocate later
legs — leg order must not affect the verdict); and **redirect, never pipe**
(a pipe hands `$?` to the downstream tool and reports the wrong leg's rc).

**Consequence for the leg — binding:** the builder must run this canon loop
**itself** before invoking `.fleet/gate-entry.sh`, and **must not edit the
gate entry** (the wall stands). `gate-rc/` is already gitignored
(`.gitignore:21`, pre-existing on `main`), so running it does not dirty the
tree. A leg that invokes only the entry gets `GATE_EXIT=1` with a message
about a missing count and will misread it as a red gate.

---

## V2 — ⛔ §7 AMENDED, LOAD-BEARING: the baseline gate is ALREADY RED on this venue — **five of nine legs**, on the untouched tree.

Flight 1 measured `cargo check` (0 errors — this seat re-confirmed it). It
never ran the gate. This seat ran **the full nine-leg canon loop on the
untouched tree** (the V1 recipe), then graded it with the unedited
`.fleet/gate-entry.sh`.

OBSERVED — the tree's Rust source is **byte-identical to beamr `main`**:
`git diff --name-only c55ac360..HEAD` returns only `.fleet/*` and
`.gitignore`. **Nothing in this flight's base caused any of this.**

### The baseline, per leg (`gate-rc/<leg>.rc`, untouched tree)

| # | Leg | rc | Verdict | Cause — OBSERVED |
|---|---|---|---|---|
| 1 | `fmt` | 0 | **green** | — |
| 2 | `clippy` | **101** | RED | 15 lint errors: `native/file_meta_bifs.rs` ×8 (`clippy::unnecessary_cast`, `u32`→`u32`, `:251`-`:264`), `io/uring.rs` ×7 (`unused_imports` `:10`, `dead_code` `:55`,`:63`, `clippy::io_other_error` `:138`, collapsible-if ×2, `while let` ×1) |
| 3 | `wasm32-check` | 0 | **green** | — |
| 4 | `wasm-tests` | 0 | **green** | 86 passed |
| 5 | `tests` | **101** | RED | **1 test fails: `crates/beamr/tests/thread_inventory_distribution.rs:59`** — `an owned distribution bundle must build both runtime workers: dist-send=1, net-kernel=0`. 2126 passed / 1 failed across 66 binaries |
| 6 | `blocking-call-in-native-bif` | 0 | **green** | — |
| 7 | `clippy-all-features` | **101** | RED | same lint family under `--all-features` |
| 8 | `tests-all-features` | **101** | RED | First run died on `error: rustc interrupted by SIGBUS` in `LLVMContextImplD1Ev` — **transient, not reproduced**. A clean re-run compiled past it and then failed on **the same single test as leg 5** (66 result lines, 1 failed, rc=101) |
| 9 | `nostd-ratchet` | **3** | RED — **INSTRUMENT DEAD ON THIS VENUE** | `mktemp: too few X's in template 'nostd-ratchet'` |

Grader verdict: `ENTRY_EXIT=1`, stopping at
`TEST-COUNT: tests-all-features emitted no 'test result:' line` (leg 8 never
reached a test binary on the first run because of the SIGBUS).

### Five red legs, but only THREE root causes

| Root cause | Legs | In this flight's scope? |
|---|---|---|
| 15 pre-existing clippy lints in `file_meta_bifs.rs` + `io/uring.rs` — **15 in both legs, identical count** | 2, 7 | **No** |
| **One** deterministic test failure: `thread_inventory_distribution` | 5, 8 | **No** |
| `mktemp` template incompatibility — the ratchet instrument is dead here | 9 | **No** |

**Not one of the three is in the four accessor families or reachable from
them.** That is the whole point of measuring the baseline: after the fix, this
table is the control.

### Leg 9 — the ratchet cannot run here at all

OBSERVED — `scripts/gate-nostd-ratchet.sh:182`: `LOG="$(mktemp -t nostd-ratchet)"`.
GNU coreutils **9.10** (this venue) requires a template ending in ≥3 `X`s;
this seat reproduced it directly: `mktemp -t nostd-ratchet` → *"too few X's in
template"*, rc=1. The script's `$LOG` is then empty, `cargo check` writes
nowhere, `parse_tally` reads nothing, and the script's **own REFUSE arm fires
correctly**:

```
REFUSE: cargo exited 1 but no "due to N previous errors" line was
  found. This gate cannot measure, so it does not get to report.
```

That is the harness **working as designed** — refusing to green falsely.
Consequence: **§7's trap #1 is moot on this venue.** The leg cannot measure
the no-std tally, so it cannot lower `CEILING` even if its change legitimately
improves it. `gates.json` declares **no** `cannot_measure_rcs` for any leg
(`jq` OBSERVED), so under `ci-verdict.sh`'s undeclared contract rc=3 is a
**FAIL**, not a CANNOT-MEASURE class. **The leg does not fix this script** —
it is the repo owner's instrument, out of scope, and touching it is not this
flight's business. Report it.

### Leg 5 — a genuine, deterministic red on `main`, and it is NOT an intermittent

OBSERVED — this seat ran the failing test **6 times: 6 failures**, identical
message each time (5× inside the sandbox, 1× with the sandbox disabled — so
it is **not** a sandbox artifact either).

OBSERVED — it is **not** in `docs/KNOWN-INTERMITTENTS.md`, and that registry's
own rule forecloses the shrug anyway: *"An entry here does **not** license
dismissing a red: if a test in this file fails a gated run, that run is RED."*

Mechanism OBSERVED, root cause NOT proven: `distribution/mod.rs:191-197`
builds the net-kernel tokio runtime with `builder.build().ok()` — the error is
**swallowed**, so `runtime` is `None`, and `worker_thread_names()` (`:221-233`)
returns empty, which is exactly `net-kernel=0`. Its sibling `dist-send`
built fine (`dist-send=1`). Why the second runtime fails to build here is
**not established by this seat** and is **out of scope** either way.

**Disposition: a measured red beyond this scope → REPORTED, not chased**, per
the brief's own rule. It is not in the four accessor families and not
reachable from them.

### Leg 8 — the SIGBUS was transient; the real red is leg 5's test again

OBSERVED — this seat re-ran `cargo test --workspace --all-features` clean:
**no SIGBUS**, 66 `test result:` lines, `RERUN_EXIT=101`, and the only failure
is `distribution_runtimes_exist_only_when_configured_and_are_joined_at_shutdown`
— **the same test as leg 5**.

So the SIGBUS is a crash **in the compiler**, in LLVM context teardown, not a
diagnostic about this tree; the venue has 463 G free disk and 24 G available
memory, so it is not simple exhaustion. This matches the gate's standing
rider: **an rf-only red cross-checks before belief.** The leg **re-runs leg 8
on any SIGBUS** rather than reporting it as a tree defect — and should expect
leg 8's steady-state red to be leg 5's, not a separate one.

### ⛔ Disposition — binding, and it follows the brief's own rules

1. **The leg captures its OWN baseline before the first edit** — run the V1
   canon loop on the untouched tree, keep `gate-rc/` per-leg rcs. Do not
   inherit this table on faith; refs, toolchains and venues move.
2. **The gate obligation is DIFFERENTIAL.** "Must run green" cannot mean an
   all-green grader verdict here, because the grader is already red on nine
   untouched legs' worth of evidence before this flight writes a line. The
   flight is responsible for **adding no red**: every leg green at baseline is
   green at the end, `clippy`'s error set is **unchanged in content and
   count**, and `tests` shows the **same one** failure and no other.
3. **Do NOT fix any of it.** `file_meta_bifs.rs`, `io/uring.rs`,
   `thread_inventory_distribution.rs` and `gate-nostd-ratchet.sh` are all out
   of scope. **Do NOT `#[allow]` anything** — suppressions are banned in any
   spelling. **Do NOT edit the gate entry or the gate scripts.**
4. **Report baseline and final side by side, per leg**, in R3. A leg that
   reports "gate green" without a baseline is reporting something it cannot
   have measured; a leg that reports "gate red" without one will blame its own
   change for `file_meta_bifs.rs`.
5. This is exactly the shape the brief's **measured-red wall** anticipates.
   An honest differential result — "the four families are retired, the
   compiler enforces the bound, and the gate is red on the same five legs it
   was red on before I started" — **is a success outcome**, reported as such,
   even when the workflow's grammar renders it `gates_red/Failed`. **Do not
   manufacture a green.**

---

## V3 — §3 AMENDED: "every path that can move or free heap memory requires `&mut`" has one measured exception, and it lands on candidate mechanism T1.

OBSERVED — `crates/beamr/src/gc/mod.rs:310`:

```rust
pub(crate) fn release_all_refcounted_resources_in_heap(heap: &Heap) {
```

It **frees** the `Arc<Vec<u8>>` buffers behind ProcBins — `release_proc_bin_arc(ptr)`
— through a **shared** `&Heap`, walking with `Heap::visit_boxed_objects(&self, …)`
(`process/heap.rs:452`). Its sibling `release_refcounted_resources_in_young`
(`:282`) takes `&mut Process` but reaches the same raw pointers via
`process.heap()` → `&Heap`. The intuition that `&Heap` is read-only does not
hold anywhere in `gc/`.

OBSERVED — sole caller: `crates/beamr/src/replay/driver.rs:34`, inside
`impl Drop for DecodedHeaps` (`:31`-`:37`), over the replay loader's decoded
scratch heaps.

REASONED — in **that** path ownership closes the hole: the `Vec<Heap>` is
being dropped, which requires ownership, so no `&Heap` borrow into it can be
live. The plan's conclusion that the strong tie is reachable **still stands**.
But the **signature** is a `&Heap`-reachable free, so:

- **T1 (tie to `&Heap`) must address this explicitly in R1(d).** A `&Heap`
  tie is enforced against `&mut Heap` operations, and this is a free that
  needs no `&mut Heap`. R1 must either argue the drop-ownership closure above
  or reject T1 for it.
- **T2 (tie to `&Process`) is untouched by it** — the replay scratch-heap
  path holds no `Process` at all.

This is design input for R1, **not** a scope change: `release_*` is not one
of the four accessor families and the leg does not modify it.

---

## V4 — §8 AMENDED: **134** files exceed 500 lines, not 3.

OBSERVED — `find crates -name '*.rs' -exec wc -l {} + | awk '$1>500 && $2!="total"' | wc -l`
→ **134**. The largest: `scheduler/tests.rs` 5165, `beamr-wasm/src/lib.rs`
4625, `jit/compiler/compiler_tests.rs` 3949, `interpreter/tests.rs` 2837,
`native/context/mod.rs` **1815** (this one is in the T4 blast radius).

§8's list of three is the subset in this flight's likely blast radius, which
is the useful list — but the wall cannot mean "no file in this repo exceeds
500 lines", because 134 already do, on `main`, untouched. **Operational
meaning for the leg:** do not create a new over-500 file, and do not grow a
file you touch past 500. `accessors.rs` at 472 remains the live risk exactly
as §8 says, and the module split should still be planned up front.

---

## V5 — §10 AMENDED: the ancestor probe now returns four hits, and all four are this workflow's own `fleet/*` refs.

OBSERVED — re-run on this tree: the eight divergent working branches and
**every ahead/behind count are identical** to §10. `fix/0163-borrow-across-alloc`
is still 401 behind / 15 ahead, merge-base still `67f89c41`, and
`git diff --stat main...origin/fix/0163-borrow-across-alloc -- crates/beamr/src/term/`
is still **empty**.

OBSERVED — but `git merge-base --is-ancestor main <ref>` is now TRUE for four
refs, **all of them `fleet/*`**: `refs/heads/fleet/accessor-lifetimes-base`,
`refs/remotes/origin/fleet/accessor-lifetimes-base`,
`refs/remotes/origin/fleet/beamracc-flight1-flagged`, and
`refs/remotes/origin/HEAD`. Flight 1's rescued branch and this flight's own
base did not exist when §10 was measured.

§10's finding — *`main` is an ancestor of no divergent WORKING branch* —
**still holds**. But the leg's re-run **must exclude `refs/*/fleet/*` and
`origin/HEAD`**, or it will report this workflow's own branches as "the
divergent working branch" the owner has to re-merge into. That would be the
stale-shape error the gate's binding note 2 exists to prevent, arrived at
from the opposite direction.

---

## V6 — §2 ADDENDUM: two benign `'static` sites the classification did not name.

OBSERVED — workspace census `rg -n "fn .*->.*&'static" crates/ --glob '*.rs'`
→ 60 sites total. Every non-`&'static str` site maps to a plan class **once
these two are added**:

| Coordinate | Why benign |
|---|---|
| `crates/beamr/src/native/stdlib_stubs/encoding_bifs.rs:83` `base64_alphabet(mode, context) -> Result<&'static [u8; 64], Term>` | A genuine `'static` constant (the base64 alphabet). **It sits in the same file as Class C `:203`** — the leg sweeping `encoding_bifs.rs` must not "fix" it. |
| `crates/beamr/src/jit/runtime_closure.rs:144` `runtime_cache(context) -> Option<&'static JitCache>` | The JIT cache, not process-heap memory. Class E. |

The mandated sweep is scoped to `crates/beamr/src/term/`, where the only
benign residue remains the two `&'static str` literals at `json.rs:22`,`:34`
exactly as §1 and the leg brief say. These two are named so the **workspace**
census has no unexplained residue either.

---
# PLAN — beamr accessor lifetimes

Authored by flight 1's planning counsel; **carried and verified whole for flight
`beamracc-lifetimes-flight3`** (see the verification pass above). §1–§12 are
flight 1's text, preserved.

Planning counsel, 2026-08-22. Tree: `fleet/accessor-lifetimes-base`
(`0013313c` = `main` `c55ac360` + the .fleet documents). Brief of record:
`.fleet/BRIEF.md`, read whole before this plan was written.

Every coordinate below is **re-derived on this tree**, not copied from the
brief or the audit. Claims are marked OBSERVED (command output / file:line)
or REASONED.

---

## 1. The mandated sweep, run on this tree

OBSERVED — `rg -n "'static" crates/beamr/src/term/`, verbatim listing:

```
crates/beamr/src/term/shared_binary.rs:79:    pub(crate) fn bytes_from_raw_word(raw: u64) -> &'static [u8] {
crates/beamr/src/term/json.rs:22:    UnsupportedTerm(&'static str),
crates/beamr/src/term/json.rs:34:    AllocationFailed(&'static str),
crates/beamr/src/term/binary.rs:70:    pub fn as_bytes(self) -> &'static [u8] {
crates/beamr/src/term/binary_ref.rs:30:    pub fn as_bytes(&self) -> &'static [u8] {
crates/beamr/src/term/compare/mod.rs:337:fn binary_bytes(term: Term) -> &'static [u8] {
crates/beamr/src/term/compare/mod.rs:377:fn normalized_limbs(bigint: BigInt) -> &'static [u64] {
crates/beamr/src/term/boxed/accessors.rs:113:    pub fn limbs(self) -> &'static [u64] {
crates/beamr/src/term/boxed/accessors.rs:281:    pub fn as_bytes(self) -> &'static [u8] {
crates/beamr/src/term/boxed/accessors.rs:340:    pub fn as_bytes(self) -> &'static [u8] {
crates/beamr/src/term/boxed/accessors.rs:357:fn parent_bytes(parent: Term) -> Option<&'static [u8]> {
```

**The gate's five are all present at the briefed coordinates**, `:357`
included — no line drift between `origin/main` at the gate and this tree:

| # | Coordinate | Signature | Family |
|---|---|---|---|
| 1 | `crates/beamr/src/term/boxed/accessors.rs:113` | `pub fn limbs(self) -> &'static [u64]` | BigInt |
| 2 | `crates/beamr/src/term/boxed/accessors.rs:281` | `pub fn as_bytes(self) -> &'static [u8]` | ProcBin |
| 3 | `crates/beamr/src/term/boxed/accessors.rs:340` | `pub fn as_bytes(self) -> &'static [u8]` | SubBinary |
| 4 | `crates/beamr/src/term/boxed/accessors.rs:357` | `fn parent_bytes(parent: Term) -> Option<&'static [u8]>` | SubBinary (private helper) — **the gate's KNOWN ANSWER** |
| 5 | `crates/beamr/src/term/shared_binary.rs:79` | `pub(crate) fn bytes_from_raw_word(raw: u64) -> &'static [u8]` | SharedBinary |

**The sweep also returns four sites the gate's list did not name, and two
benign ones.** The known answer is a floor, not a ceiling. Classification
below is the load-bearing planning output of this flight.

`json.rs:22` and `json.rs:34` are `&'static str` **error-message literals** —
genuinely `'static`, not over heap memory. Out of the defect class. The leg
states this in the sweep, rather than letting an unexplained residue stand.

---

## 2. Classification of every `'static`-over-memory site (workspace-wide)

⚠ AMENDED — see **V6** (two benign sites this classification did not name).

OBSERVED — `rg -n "fn .*->.*&'static" crates/ --glob '*.rs'`, then each site
read and its provenance traced by hand.

### Class A — the four families at the source. IN SCOPE, must be retired.
The five in the table above.

### Class B — in-family members the gate's list did not name. IN SCOPE: the tie cannot complete without them.

| Coordinate | Signature | Why it is in-family |
|---|---|---|
| `crates/beamr/src/term/binary.rs:70` | `Binary::as_bytes(self) -> &'static [u8]` | Inline heap binary. **`parent_bytes` calls it directly** (`accessors.rs:359`: `Some(binary.as_bytes())`). `parent_bytes` cannot be lifetime-tied while its own callee still hands out `'static` — the only way to keep `Binary::as_bytes` at `'static` and tie `parent_bytes` is to launder, which R2 forbids. |
| `crates/beamr/src/term/binary_ref.rs:30` | `BinaryRef::as_bytes(&self) -> &'static [u8]` | The public fan-in over all three binary families (`binary_ref.rs:31-35` dispatches to `Binary`/`ProcBin`/`SubBinary` `as_bytes`). It is the accessor **most callers actually use** — 94 `BinaryRef::new` sites across 47 files. Leaving it `'static` preserves the defect for the majority of the call graph while the four "source" signatures read as fixed. |

REASONED: Class B is not scope creep. `Binary::as_bytes` is reached *by*
`parent_bytes` and `BinaryRef::as_bytes` re-exports all three; both sit
inside the brief's own boundary, `term/`. The brief's criterion — "zero
remaining `'static` returns over heap memory in the four families" — is not
met while either survives.

### Class C — compiler-forced consumers. IN SCOPE by construction: they cannot compile as `'static` once A+B are tied.

Each derives its bytes from a Class A/B accessor. After the tie, each is a
type error unless laundered — this is the no-laundering rule doing its work,
and it is how the compiler produces the caller inventory for free.

| Coordinate | Derives from |
|---|---|
| `crates/beamr/src/term/compare/mod.rs:337` `binary_bytes` | `BinaryRef::as_bytes` |
| `crates/beamr/src/term/compare/mod.rs:377` `normalized_limbs` | `BigInt::limbs` |
| `crates/beamr/src/interpreter/opcodes/binary/matching.rs:806` `slice` | `self.source()?.as_bytes()` → `BinaryRef` (`matching.rs:780`) |
| `crates/beamr/src/jit/runtime_binary_match.rs:212` `slice` | `self.source()?.as_bytes()` → `BinaryRef` — **the JIT path; Tom's ruling that jit is not optional bites exactly here** |
| `crates/beamr/src/native/stdlib_stubs/string_bifs.rs:355` `binary_bytes` | `BinaryRef::as_bytes` |
| `crates/beamr/src/native/stdlib_stubs/string_bifs.rs:351` `utf8_str` | the line above |
| `crates/beamr/src/native/stdlib_stubs/encoding_bifs.rs:203` `binary_bytes` | `BinaryRef::as_bytes` |
| `crates/beamr/src/native/gate3_bifs/type_conversion.rs:230` `binary_to_utf8` | `BinaryRef::as_bytes` |
| `crates/beamr/src/native/otp_stubs/tests.rs:407` `binary_bytes` | `BinaryRef` (test code — still must compile) |

`string_bifs.rs` is the file whose five BIFs were the 0.16.2/0.16.3
call-site sweep. Its two `'static` helper signatures survived that sweep
untouched — OBSERVED at `:351` and `:355`. That is the brief's thesis
demonstrated in the tree: **the sweep fixed sites and left the signatures
that manufacture the next unsound caller.**

### Class D — independent `'static` manufacture over heap memory, NOT reached through the four families. MEASURED RED BEYOND SCOPE → REPORTED, NOT CHASED.

| Coordinate | Why not compiler-forced |
|---|---|
| `crates/beamr/src/interpreter/opcodes/binary/mod.rs:91` `slice_from_words` | Calls `std::slice::from_raw_parts` on a raw heap pointer directly. Owes nothing to the accessors; the tie will not break it. |
| `crates/beamr/src/interpreter/opcodes/binary/construction.rs:213` `BinaryBuilder::bytes` | Calls `slice_from_words` (above). |
| `crates/beamr/src/jit/runtime_binary_build.rs:193` `bytes` | Its own `from_raw_parts` over the builder pointer. |

REASONED: these are the **binary-construction** family, not the four
**accessor** families the brief names. Same defect class, different source.
The brief is explicit — "A measured red beyond this scope is REPORTED, not
chased" — so the leg reports them with coordinates in R3 and does not fix
them. **Conditional promotion:** if a Class D site turns out to consume a
Class A/B accessor after all, the compiler will say so, and it becomes
in-scope automatically. The leg follows the compiler, not this table.

### Class E — `'static` over non-process-heap memory. Not the defect; named so the sweep has no unexplained residue.

`term/json.rs:22,:34` (`&'static str` literals); `atom/table.rs:241`
(`Box::leak` — genuinely `'static`); `io/resource.rs:278` (FD table);
`jit/runtime.rs:400` and `jit/ir_exceptions.rs:343`
(`process_from_abi -> Option<&'static mut Process>` — the JIT ABI raw-pointer
boundary; a real and separate soundness question, **not** one of the four
families, out of scope, report only); and the many `-> &'static str` label
functions across `error.rs`, `telemetry/`, `scheduler/`, `beamr-wasm`.

---

## 3. Ownership structure — is the strong tie reachable?

⚠ AMENDED — see **V3** (one measured `&Heap`-reachable free; bears on T1).

This is the question that decides whether the flight delivers a fix or a
measured-red conclusion. **OBSERVED: it is reachable.** The allocation and
collection surface is uniformly `&mut`:

| Coordinate | Signature |
|---|---|
| `crates/beamr/src/process/heap.rs:285` | `Heap::alloc(&mut self, words) -> Result<*mut u64, HeapFull>` |
| `crates/beamr/src/process/heap.rs:298` | `Heap::alloc_slice(&mut self, words)` |
| `crates/beamr/src/process/mod.rs:440` | `Process::heap(&self) -> &Heap` |
| `crates/beamr/src/process/mod.rs:445` | `Process::heap_mut(&mut self) -> &mut Heap` |
| `crates/beamr/src/gc/mod.rs:135` | `gc::alloc(process: &mut Process, words)` |
| `crates/beamr/src/gc/mod.rs:149` | `gc::ensure_space(process: &mut Process, words, live_x)` |
| `crates/beamr/src/gc/mod.rs:101` | `gc::collect_minor(process: &mut Process)` |
| `crates/beamr/src/gc/mod.rs:118` | `gc::collect_major(process: &mut Process)` |
| `crates/beamr/src/process/gc.rs:43` | `root_set(process: &mut Process, live_x)` |

REASONED: **every path that can move or free heap memory requires `&mut
Heap` or `&mut Process`.** A slice whose lifetime is tied to a shared borrow
of either therefore makes "hold the slice across a GC-triggering allocation"
a borrow-checker conflict — E0502/E0499/E0505 — not a runtime check. That is
precisely the brief's R1(d): the interaction becomes **unrepresentable**,
because the `&mut` the collector needs cannot be produced while the shared
borrow lives. No generation counter, no debug-assert, no rooting registry is
required for the compiler to enforce it.

One subtlety the leg must not miss: `SharedBinary::bytes_from_raw_word`
points at `Arc<Vec<u8>>` memory, which does **not** move on GC — a naive read
concludes `'static` is harmless there. It is not. The GC *releases* the Arc:
`gc/mod.rs:282` `release_refcounted_resources_in_young`, `:310`
`release_all_refcounted_resources_in_heap`, `:321`
`release_all_refcounted_resources`. When the owning ProcBin dies in a
collection the strong count drops and the `Vec` can be freed under the
slice. The bound is the **ProcBin's liveness**, which is the heap's — so the
same `&Heap`/`&Process` tie is the right one.

### The guard that already exists

`crates/beamr/src/native/context/mod.rs:337`:
`pub struct ProcessContext<'process> { … process: Option<&'process mut Process>, … }`,
with every allocator taking `&mut self` (`context/alloc.rs:24-288`,
`context/mod.rs:1767`). The BIF layer — where most callers live — already
carries a lifetime-parameterised guard over `&mut Process`. It is a
candidate tie point, but see T4 below for why it cannot be the sole one.

---

## 4. Viable lifetime-tying mechanisms (the leg chooses in R1; this maps the terrain)

- **T1 — tie to `&Heap`.** `fn limbs<'h>(self, heap: &'h Heap) -> &'h [u64]`.
  Narrowest witness. Conflicts with `Heap::alloc(&mut self)` and with
  `Process::heap_mut`. Cost: every call site must have a `&Heap` in hand;
  `term/` internals (`compare`, `hash`, `json`, `etf`) currently do not.
- **T2 — tie to `&Process`.** `fn limbs<'p>(self, process: &'p Process) -> &'p [u64]`.
  Conflicts with the whole `&mut Process` collector surface, which is the
  bound that actually matters. Same witness-threading cost, one level up.
- **T3 — lifetime-parameterise the accessor struct, witness at construction.**
  `BigInt<'p>::new(term, &'p Process) -> Option<BigInt<'p>>`, then
  `limbs(self) -> &'p [u64]`. Callers pay the witness **once**, at
  construction, instead of at every accessor call — materially smaller diff
  across 47 `BinaryRef` files, and it ties `len()`/`is_empty()` coherently
  too. Likely the best cost/strength point; the leg must verify it against
  the real call sites.
- **T4 — tie to `&ProcessContext<'_>`.** Natural for the BIF layer, but
  `compare/`, `hash/`, `json/`, `etf/`, the interpreter and the JIT runtime
  hold no `ProcessContext`. Cannot be the single mechanism; may be a
  convenience layer above T1/T2/T3.
- **T5 — lifetime-parameterise `Term<'h>` itself.** Strongest and most
  uniform; blast radius across essentially the whole crate. If the leg
  concludes the tie needs T5, that conclusion with its evidence **is** the
  deliverable under the measured-red wall — committed and reported, no
  heroics.

### Alternatives the R1 artifact must address and reject on the record
- **Debug-asserts / generation counters.** Explicitly rejected by Tom's
  ruling: release builds hold the real data. May exist as defence-in-depth;
  they are not the criterion.
- **Return owned `Vec<u8>` / copy out.** Removes the dangle but changes the
  cost model of every binary BIF, and performance work is out of scope in
  both directions — a silent copy is not a lifetime fix.
- **`&self`-elision alone** (`fn limbs(&self) -> &[u64]`). Ties the slice to
  a borrow of the *accessor struct*, which is a `Copy` raw pointer living on
  the caller's stack — **not** to the heap. It kills the
  `BigInt::new(t).limbs()` temporary pattern, which is a genuine
  improvement, but a caller who binds `let b = BigInt::new(t)?;` then holds
  `b.limbs()` across an allocation still compiles. **Insufficient alone**;
  the leg must say so rather than shipping it as the tie.

### ⛔ The failure mode that would produce a fake fix — name it, wall it
REASONED: a lifetime parameter that appears **only** in the return type or
in a `PhantomData` field, with no witness in **argument** position, is
unconstrained — the caller's inference picks whatever lifetime it needs,
including `'static`. That passes the `'static` sweep and keeps the bug
exactly. It is lifetime laundering by inference, and it is the single most
likely way this flight produces a green that means nothing. **The witness
borrow must appear in an argument position** (or in the constructor that
produces the accessor, T3). The leg brief carries this as a wall.

---

## 5. API breakage and blast radius

OBSERVED — all four families are **public API**: `lib.rs:62` `pub mod term;`
→ `term/mod.rs:10,11,12` `pub mod binary; pub mod binary_ref; pub mod boxed;`
→ `term/boxed/mod.rs:12` `pub use accessors::{BigInt, Closure, Cons,
ExternalPid, ExternalReference, Float, Map, ProcBin, Reference, SubBinary,
Tuple};`. Every signature change is a **breaking change to the `beamr`
crate's public surface**. `SharedBinary::bytes_from_raw_word` is
`pub(crate)` — internal only.

OBSERVED — pre-implementation caller estimate (grep, whole workspace incl.
tests and jit):

| Pattern | Sites | Files |
|---|---|---|
| `BinaryRef` (all uses) | 156 | 47 |
| `BinaryRef::new` | 94 | 47 |
| `Binary::new` | 111 | 39 |
| `BigInt::new` | 37 | 22 |
| `.limbs()` | 44 | 22 |
| `ProcBin::new` | 23 | 14 |
| `SubBinary::new` | 14 | 8 |
| `parent_bytes` | 4 | 1 |
| `bytes_from_raw_word` | 2 | 2 |

JIT files touched: `jit/runtime.rs`, `jit/runtime_binary_build.rs`,
`jit/runtime_binary_match.rs`, `jit/runtime_map.rs`, `jit/aot.rs`.

⚠️ These grep counts are a **pre-estimate, not the inventory**. `.limbs()`
is overloaded — `BigIntValue::limbs` on the owned type
(`compare/mod.rs:391`) is a different method — and `.as_bytes()` collides
with `str`/`String`/`SharedBinary`. **The authoritative caller inventory is
the compiler's error list after the signatures change**, which is the honest
instrument and the one R1(b)/R3 should be built on. The leg states the
method, not just the number.

---

## 6. Instruments verified on this venue

- **miri: PRESENT.** OBSERVED `miri 0.1.0 (8925ea358a 2026-08-20)`.
  ⚠️ Only after `export PATH="$HOME/.cargo/bin:$PATH"` — the default PATH
  reaches Arch system rust 1.94.0 at `/usr/bin` and `rustup` is not on it, so
  a bare `cargo +nightly miri` fails with *"no such command: `+nightly`"* and
  reads as "miri absent". `.fleet/gate-entry.sh` already does this export;
  the leg must too, in every shell. Toolchains present: stable, nightly,
  1.94.0, **1.97.1 (active — pinned by `rust-toolchain.toml`)**.
- **compile-fail harness: ALREADY IN THE REPO.** OBSERVED
  `crates/beamr/src/process/mod.rs:146` and `:155` — rustdoc
  ` ```compile_fail ` doctests, used for exactly this kind of negative
  type-level proof (that `Process` is neither `Send` nor `Sync`). Doctests
  are enabled (no `doctest = false` in any manifest) and run under the
  gate's `tests` leg. **This is the zero-new-dependency, in-gate mechanism
  for the compile-fail green evidence**, and it satisfies the brief's
  "trybuild-style compile-fail test only if the repo already carries that
  harness". No `trybuild` dependency is needed or permitted.
  ⚠️ A bare `compile_fail` passes if the snippet fails for *any* reason —
  including a typo. The leg must pin the expected diagnostic
  (` ```compile_fail,E0502 ` or the applicable E0499/E0505) so the proof is
  that **the borrow** is rejected, not that a name was misspelled.
  ⚠️ Doctests only reach the **public** API. `parent_bytes` is private, so
  its compile-fail proof runs through the public `SubBinary::as_bytes` path.

---

## 7. Gate reality — `gates.json`, nine legs, two of them traps

⚠ AMENDED — see **V1** (the entry grades, it does not run the legs) and **V2**
(the baseline gate is already RED: `clippy` = 15 pre-existing errors).

The final gate is the repository's own harness: `./scripts/ci-verdict.sh
gate-rc gates.json` via `.fleet/gate-entry.sh`. Nine legs: `fmt`, `clippy`,
`wasm32-check`, `wasm-tests`, `tests`, `blocking-call-in-native-bif`,
`clippy-all-features`, `tests-all-features`, `nostd-ratchet`.

Two legs will bite this specific change and the leg must expect them:

1. **`nostd-ratchet` is strict in BOTH directions.** OBSERVED
   `scripts/gate-nostd-ratchet.sh:81` `CEILING=1072`; `:120` fails if the
   tally *exceeds* it, `:127` fails if the tally is *below* it, instructing
   `Set CEILING=$tally in this script, in the same commit`. Changing `term/`
   signatures will very likely move the `--no-default-features` error tally.
   REASONED disposition: a tally that goes **down** is legitimate — lower the
   ceiling in the same commit, exactly as the script instructs. A tally that
   goes **up** is new no-std breakage and must be fixed, **never** absorbed
   by raising the ceiling (`:81` "LOWER THIS, NEVER RAISE"). Count with
   rustc's own "due to N previous errors" line, not `grep -c '^error'` — the
   script's own note records that off-by-one trap.
2. **`clippy` and `clippy-all-features` run `-D warnings` across three
   distinct feature configurations.** Lifetime parameters commonly draw
   `needless_lifetimes`, `elidable_lifetime_names`, `extra_unused_lifetimes`.
   The wall forbids suppressions in any spelling — the leg fixes the code,
   never `#[allow]`s the lint.

---

## 8. Hard-wall risks specific to this tree

⚠ AMENDED — see **V4** (134 files already exceed 500 lines, not 3).

- ⚠️ **`accessors.rs` is at 472 lines — 28 from the 500-line wall.** OBSERVED
  `wc -l crates/beamr/src/term/boxed/accessors.rs` = 472 (348 non-blank,
  non-comment). It is the most-edited file of the flight, and lifetime
  parameters plus **re-derived safety comments on every touched `unsafe`
  block** will exceed 28 lines. **The leg should plan the module split up
  front** (e.g. the binary family out of `accessors.rs`) rather than
  discovering the wall at the end.
- Pre-existing over-500 files the leg must **not** grow:
  `term/compare/mod.rs` 713, `process/mod.rs` 1361,
  `native/stdlib_stubs/gc_rooting_tests.rs` 552. These pre-date the flight;
  report as measured, do not chase.
- **No `.unwrap()`/`.expect()` in library code.** Note the current
  `SubBinary::as_bytes` uses `unwrap_or`/`checked_add` correctly
  (`accessors.rs:341-344`) — keep that discipline through the rewrite.
- **No lint suppressions added.** Pre-existing ones (e.g.
  `gc_rooting_tests.rs:27` `#[allow(clippy::cast_precision_loss)]`) are not
  this flight's business unless it edits that line.

## 9. The red — precedent already in the tree

`crates/beamr/src/native/stdlib_stubs/gc_rooting_tests.rs` is the model: it
"force[s] collections in the middle of BIF allocation sequences with boxed
(heap-pointer) terms live" using a deliberately small heap
(`Process::new(1, 96)`, `:37`). The dangle demonstration follows the same
shape — small heap, obtain the accessor slice, force the collection, read
through the stale slice — with miri as the instrument that turns "stale
bytes" into a hard diagnosis.

## 10. Follow-on flag — measured, and the audit's shape is dead

⚠ AMENDED — see **V5** (the ancestor probe now returns four `fleet/*` refs; exclude them).

OBSERVED — `git for-each-ref` + `git rev-list --left-right --count main...<ref>`
over all local and remote refs:

```
origin/fix/0163-borrow-across-alloc      behind_main=401  ahead_of_main=15
origin/diana/seat-preserve-annabelbox    behind_main=270  ahead_of_main=4
origin/artemis/backport-0.17.1           behind_main=204  ahead_of_main=3
origin/seth/replay-rebuild-3aecb622      behind_main=296  ahead_of_main=3
origin/diana/beamr-wedge002-3aecb622     behind_main=280  ahead_of_main=3
origin/artemis/jit-operator-switch       behind_main=23   ahead_of_main=2
origin/feat/jit-cache-enumeration        behind_main=18   ahead_of_main=1
origin/juniper/docs-package              behind_main=526  ahead_of_main=1
```

OBSERVED — `git merge-base --is-ancestor main <ref>` returns false for every
one of them: **`main` is an ancestor of NO divergent branch.** No ref is
"173 ahead with main an ancestor" — the audit's shape describes nothing in
this repository, exactly as gate binding note 2 anticipated.

The topical branch is **`origin/fix/0163-borrow-across-alloc`** — the prior
0.16.3 call-site-sweep lane for this very defect, 401 behind / 15 ahead,
merge-base `67f89c41`. OBSERVED: `git diff --stat main...origin/fix/0163-borrow-across-alloc
-- crates/beamr/src/term/` is **empty** — it carries no accessor changes to
re-merge. The leg re-runs this measurement at flight time (refs may move) and
reports the result; it does not repeat the audit's number.

## 11. Sequence

1. **R1 design artifact** `docs/design/accessor-lifetimes.md` — committed
   **before** any signature moves. Mechanism chosen + alternatives rejected
   (§4), caller inventory (§5 method), API breakage (§5), unrepresentability
   argument (§3).
2. **The red** — dangle demonstrated, instrument named, captured verbatim to
   `docs/evidence/beamr-accessor-dangle-red.txt`. Committed **before** R2.
3. **R2 implementation** — signatures tied; every caller compiles against the
   new bounds; no transmute, no `from_raw_parts` re-manufacture, no
   unconstrained lifetime; every touched `unsafe` block's safety comment
   re-derived against the NEW lifetimes.
4. **R3 proof + report** — re-run sweep with output, `:357` in the
   found-and-retired list, compile-fail evidence per family, API-breakage
   list, follow-on flag by measurement, full gate green.

Report lands byte-exact at **`.fleet/reports/accessor.md`**, confirmed with
`git ls-tree HEAD -- .fleet/reports/`.

## 12. Lane boundary

This flight produces a **candidate** result branch. Nothing merges to beamr
`main` at any seat in this workflow; the landing is ratified by the repo
owner's seat. No version bump, no changelog, no release prep.
