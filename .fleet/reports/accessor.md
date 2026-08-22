Model: `claude-opus-5` — this seat's own identifier, verbatim. Pin verified
in-band, not assumed: `env | grep ANTHROPIC_MODEL` → `ANTHROPIC_MODEL=claude-opus-5`
(OBSERVED). The harness reports the exact served model ID as `claude-opus-5[1m]`.

# REPORT — beamr accessor lifetimes, PLANNING SEAT (flight `beamracc-lifetimes-flight3`)

**Seat: planning counsel. Status: planning complete, committed.** The
implementation seat has not run. This report covers the planning duties only;
the builder's R1/R2/R3 report replaces it at this same contracted path.

Tree: `fleet/accessor-lifetimes-base` @ `85540999` (= beamr `main` `c55ac360`
+ `.fleet` documents). OBSERVED: `git diff --name-only c55ac360..HEAD` returns
only `.fleet/*` and `.gitignore` — **the Rust source is byte-identical to
beamr `main`.**

Deliverables committed: `.fleet/plan.md` (739 lines), `.fleet/legs/accessor.md`
(422 lines), at `85540999`.

---

## 1. The mandated sweep — run on this tree, by this seat

OBSERVED — `rg -n "'static" crates/beamr/src/term/`, verbatim:

```
crates/beamr/src/term/shared_binary.rs:79:    pub(crate) fn bytes_from_raw_word(raw: u64) -> &'static [u8] {
crates/beamr/src/term/binary.rs:70:    pub fn as_bytes(self) -> &'static [u8] {
crates/beamr/src/term/json.rs:22:    UnsupportedTerm(&'static str),
crates/beamr/src/term/json.rs:34:    AllocationFailed(&'static str),
crates/beamr/src/term/compare/mod.rs:337:fn binary_bytes(term: Term) -> &'static [u8] {
crates/beamr/src/term/compare/mod.rs:377:fn normalized_limbs(bigint: BigInt) -> &'static [u64] {
crates/beamr/src/term/binary_ref.rs:30:    pub fn as_bytes(&self) -> &'static [u8] {
crates/beamr/src/term/boxed/accessors.rs:113:    pub fn limbs(self) -> &'static [u64] {
crates/beamr/src/term/boxed/accessors.rs:281:    pub fn as_bytes(self) -> &'static [u8] {
crates/beamr/src/term/boxed/accessors.rs:340:    pub fn as_bytes(self) -> &'static [u8] {
crates/beamr/src/term/boxed/accessors.rs:357:fn parent_bytes(parent: Term) -> Option<&'static [u8]> {
```

**The gate's KNOWN ANSWER of five is confirmed at the briefed coordinates,
`:357` included — no line drift** between `origin/main` at the gate seat and
this tree. Each was additionally re-derived **by symbol** (read at the line,
not just matched):

| # | Coordinate | Signature | Family |
|---|---|---|---|
| 1 | `crates/beamr/src/term/boxed/accessors.rs:113` | `pub fn limbs(self) -> &'static [u64]` | BigInt |
| 2 | `crates/beamr/src/term/boxed/accessors.rs:281` | `pub fn as_bytes(self) -> &'static [u8]` | ProcBin |
| 3 | `crates/beamr/src/term/boxed/accessors.rs:340` | `pub fn as_bytes(self) -> &'static [u8]` | SubBinary |
| 4 | `crates/beamr/src/term/boxed/accessors.rs:357` | `fn parent_bytes(parent: Term) -> Option<&'static [u8]>` | SubBinary helper — **the KNOWN ANSWER** |
| 5 | `crates/beamr/src/term/shared_binary.rs:79` | `pub(crate) fn bytes_from_raw_word(raw: u64) -> &'static [u8]` | SharedBinary |

Residue in the sweep is accounted for, not left standing: `json.rs:22`,`:34`
are `&'static str` **error-message literals** — genuinely `'static`, not over
heap memory. The remaining four lines are in-family (`binary.rs:70`,
`binary_ref.rs:30`) or compiler-forced consumers (`compare/mod.rs:337`,`:377`).

---

## 2. Verification verdict on the carried plan

The carry notice directed this seat to **verify, not rewrite**. Flight 1's
plan and leg brief are carried whole; §1–§12 are preserved as authored.

**Verdict: the terrain map is accurate.** Re-derived and confirmed identical:
all 11 sweep lines; all five family coordinates; Class B/C/D/E coordinates
(2/9/3/4); the ownership surface (9 coordinates, `heap.rs:285` … `process/gc.rs:43`);
`ProcessContext<'process>` at `context/mod.rs:337`; the public re-export chain
`lib.rs:62` → `term/mod.rs:10-12` → `boxed/mod.rs:12`; **all nine grep
pre-estimate counts** (156/94/111/37/44/23/14/4/2); miri
`0.1.0 (8925ea358a 2026-08-20)`; the `compile_fail` doctest precedent at
`process/mod.rs:146`,`:155`; gates.json's nine legs; `CEILING=1072` strict in
both directions; `accessors.rs` = 472 lines; the branch measurement.

**Six corrections were required (V1–V6 in the plan). Two are load-bearing.**

---

## 3. ⛔ V1 — the gate entry does not run the legs; it only grades them

OBSERVED — `.fleet/gate-entry.sh` runs one thing: `./scripts/ci-verdict.sh
gate-rc gates.json`. OBSERVED — `scripts/ci-verdict.sh:34` declares
`Usage: ci-verdict.sh [GATE_RC_DIR] [GATES_JSON]`; it is a **grader** reading
`gate-rc/declared.count` and `gate-rc/<leg>.rc` that a separate canon step must
already have written, and `:47` returns 1 when that directory is absent.

OBSERVED — running the entry on the untouched tree: seven embedded self-tests
graded as expected, then

```
no declared-leg count: the canon step did not reach its first leg
GATE_EXIT=1
```

OBSERVED — the leg-running loop lives in **`.github/workflows/ci.yml:68-112`**,
not under `scripts/`. Its three load-bearing properties are stated in its own
comments: no `set -e`, a subshell around `eval`, and redirect-never-pipe.

**Consequence:** the builder must run the canon loop itself, then invoke the
entry — and must not edit the entry. `gate-rc/` is gitignored (`.gitignore:21`,
pre-existing). The recipe is carried verbatim into the leg brief.

---

## 4. ⛔ V2 — the baseline gate is ALREADY RED on this venue: five of nine legs

This seat ran the **full nine-leg canon loop on the untouched tree**, then
graded it with the unedited entry. Per-leg `gate-rc/<leg>.rc`:

| Leg | rc | Cause — OBSERVED |
|---|---|---|
| `fmt` | **0** | — |
| `clippy` | **101** | 15 lint errors: `native/file_meta_bifs.rs` ×8 (`clippy::unnecessary_cast`, `u32`→`u32`, `:251`-`:264`); `io/uring.rs` ×7 (`:10`,`:55`,`:63`,`:138`, +3) |
| `wasm32-check` | **0** | — |
| `wasm-tests` | **0** | 86 passed |
| `tests` | **101** | 1 test fails: `tests/thread_inventory_distribution.rs:59` — `dist-send=1, net-kernel=0`; 2126 passed / 1 failed over 66 binaries |
| `blocking-call-in-native-bif` | **0** | — |
| `clippy-all-features` | **101** | same lint family, **identical count of 15** |
| `tests-all-features` | **101** | transient `rustc interrupted by SIGBUS` (LLVM teardown) on run 1; clean re-run → **the same one test** as `tests` |
| `nostd-ratchet` | **3** | `mktemp: too few X's in template 'nostd-ratchet'` — instrument dead on this venue |

Grader: `ENTRY_EXIT=1`.

**Five red legs, THREE root causes, none of them in this flight's scope:**

1. **Pre-existing clippy lints** (legs 2+7). REASONED: platform/libc-version
   dependent — `io/uring.rs:3` is `#![cfg(target_os = "linux")]` so it compiles
   only on venues like this one, and `libc::S_IFMT as u32` is an
   `unnecessary_cast` only where this libc version already types those
   constants `u32`. This is why `main` can be green in CI and red here.
2. **One deterministic test failure** (legs 5+8). OBSERVED: run **6 times,
   failed 6 times** — 5× sandboxed, 1× with the sandbox disabled, so **not a
   sandbox artifact**. OBSERVED: **not** in `docs/KNOWN-INTERMITTENTS.md`,
   whose own rule forecloses the shrug regardless. Mechanism OBSERVED
   (`distribution/mod.rs:191-197` builds the net-kernel tokio runtime with
   `builder.build().ok()`, swallowing the error, so `worker_thread_names()`
   at `:221-233` returns empty = `net-kernel=0`); **root cause NOT established
   by this seat, and out of scope either way.**
3. **`mktemp` incompatibility** (leg 9). OBSERVED — `gate-nostd-ratchet.sh:182`
   `LOG="$(mktemp -t nostd-ratchet)"`; GNU coreutils **9.10** here requires a
   template ending in ≥3 `X`s. Reproduced directly: rc=1, *"too few X's"*. The
   script's own REFUSE arm then fires **correctly** — the harness working as
   designed, refusing to green falsely. `gates.json` declares no
   `cannot_measure_rcs` for any leg, so rc=3 grades as a plain FAIL.
   **This makes the plan's §7 ratchet trap moot on this venue**: the leg cannot
   measure the no-std tally, so it cannot lower `CEILING` even if its change
   earns it.

**Disposition carried into the leg brief as binding:** the gate obligation is
**differential**. The builder captures its own baseline before the first edit,
adds no red, fixes none of it, suppresses nothing, edits no gate script, and
reports baseline and final side by side per leg. All three root causes are
*"a measured red beyond this scope — REPORTED, not chased."*

---

## 5. V3–V6 — the remaining corrections

- **V3 (§3).** OBSERVED — `gc/mod.rs:310`
  `release_all_refcounted_resources_in_heap(heap: &Heap)` **frees** ProcBin
  `Arc` buffers through a **shared** `&Heap`, via
  `Heap::visit_boxed_objects(&self, …)` (`process/heap.rs:452`). Sole caller
  `replay/driver.rs:34`, inside `impl Drop for DecodedHeaps` (`:31`-`:37`).
  REASONED: ownership closes the hole there — dropping the `Vec<Heap>` needs
  ownership, so no `&Heap` borrow can be live — **so the strong tie remains
  reachable**. But the *signature* is a `&Heap`-reachable free, so mechanism
  **T1 (tie to `&Heap`) must address it in R1(d) or be rejected for it**;
  **T2 (tie to `&Process`) is untouched**, that path holds no `Process`.
- **V4 (§8).** OBSERVED — **134** files already exceed 500 lines on untouched
  `main`, not 3. The wall's operational meaning: create no new over-500 file,
  grow no touched file past it. `accessors.rs` at 472 remains the live risk.
- **V5 (§10).** The follow-on measurement is **unchanged**: same 8 divergent
  working branches, same counts, `origin/fix/0163-borrow-across-alloc` 401
  behind / 15 ahead, merge-base `67f89c41`, and
  `git diff --stat main...origin/fix/0163-borrow-across-alloc -- crates/beamr/src/term/`
  still **empty**. But `git merge-base --is-ancestor main <ref>` is now TRUE
  for four refs, **all `fleet/*`** (this flight's base; flight 1's rescued
  `origin/fleet/beamracc-flight1-flagged`; `origin/HEAD`) — they did not exist
  at flight 1's measurement. **The builder's re-run must exclude
  `refs/*/fleet/*` and `origin/HEAD`** or it will name this workflow's own
  branch as the owner's re-merge target.
- **V6 (§2).** Workspace census `rg -n "fn .*->.*&'static" crates/ --glob '*.rs'`
  = 60 sites; every non-`str` site maps to a plan class once two benign ones are
  added: `native/stdlib_stubs/encoding_bifs.rs:83` `base64_alphabet` (a genuine
  static constant — **in the same file as Class C `:203`**, do not "fix" it) and
  `jit/runtime_closure.rs:144` `runtime_cache` (JIT cache, not process heap).

---

## 6. Instruments confirmed for the builder

- **miri PRESENT and preferred.** OBSERVED `miri 0.1.0 (8925ea358a 2026-08-20)`
  — **only** after `export PATH="$HOME/.cargo/bin:$PATH"`; without it the
  default PATH finds Arch system rust 1.94.0 and `cargo +nightly miri` fails
  with *"no such command: `+nightly`"*, which reads as "miri absent" and is
  wrong. Active toolchain is **1.97.1**, pinned by `rust-toolchain.toml`.
- **Compile-fail harness already in-repo, zero new dependencies.** OBSERVED —
  ` ```compile_fail ` rustdoc doctests at `process/mod.rs:146` and `:155`. The
  builder pins the expected diagnostic (`compile_fail,E0502` / E0499 / E0505)
  so the proof is that *the borrow* is rejected, not that a name was
  misspelled; and still captures verbatim rustc output into `docs/evidence/`.
  Doctests reach only the **public** API, so `parent_bytes` (private) is proven
  through the public `SubBinary::as_bytes` path. **No `trybuild`.**
- `docs/design/` exists; **`docs/evidence/` does not — the builder creates it.**

---

## 7. Lane boundary and scope, restated

This flight produces a **CANDIDATE result branch**. **Nothing merges to beamr
`main` at any seat in this workflow** — the landing is ratified by the repo
owner's seat. Not in scope: performance work, new features, the divergent
working branch itself, version bumps, changelog, release.

**Follow-on flagged for the OWNER, by measurement:** re-merging the fix into
`origin/fix/0163-borrow-across-alloc` (401 behind / 15 ahead of `main`,
merge-base `67f89c41`) — the prior 0.16.3 call-site-sweep lane for this very
defect. OBSERVED: its diff against `main` over `crates/beamr/src/term/` is
empty, so it carries no accessor changes to reconcile. `main` is an ancestor
of **no divergent working branch**; the audit's "173 ahead with main an
ancestor" describes nothing in this repository, exactly as the gate's binding
note 2 anticipated. The builder re-measures at flight time.

**Measured reds reported and not chased** (beyond this flight's scope): the
three baseline-gate root causes in §4; the Class D independent `'static`
manufacture at `interpreter/opcodes/binary/mod.rs:91`,
`interpreter/opcodes/binary/construction.rs:213`,
`jit/runtime_binary_build.rs:193`; and the JIT ABI raw-pointer boundary
`process_from_abi -> Option<&'static mut Process>` at `jit/runtime.rs:400` and
`jit/ir_exceptions.rs:343`.
