# LEG BRIEF — accessor lifetimes (builder seat)

Flight `beamracc-lifetimes-flight3`. Base: `fleet/accessor-lifetimes-base`
(`main` `c55ac360` + .fleet documents). **Read `.fleet/BRIEF.md` whole
before you touch anything** — it is the brief of record and it governs.
`.fleet/plan.md` is the terrain map: coordinates, classification, gate
traps. Read it second — **including its FLIGHT 3 VERIFICATION PASS header
(V1–V6), which is not optional**: V1 and V2 change how you run the gate and
what a green means, and a leg that skips them will misread its own results.

This brief and the plan were authored by flight 1's planning counsel and are
**carried whole**. Flight 1 died at the builder seat on a **provider refusal**
— its provider declined the request under a cybersecurity policy while the
leg read unsafe-accessor content. That was not a defect in this plan and not
a judgement about this work, which is **Tom-ordered memory-safety repair** on
a VM's own term internals. Flight 3's planning seat re-derived every
coordinate below against the tree and confirmed them; see the plan's
verification table.

---

## SCOPE, VERBATIM FROM THE BRIEF, BINDING

> retire the `&'static` accessor exposure AT THE SOURCE — the accessor
> signatures — not with another call-site sweep.

**THE DEFECT:** `BigInt::limbs`, `ProcBin::as_bytes`, `SubBinary::as_bytes`
and `shared_binary`'s `bytes_from_raw_word` hand out `&'static` slices over
heap/`Arc` memory; holding one across a GC-triggering allocation dangles.

**Tom's ruling selects the strong disposition:** slices lifetime-tied to
`&Process`/`&Heap` (or the owning guard) so **the COMPILER enforces the
bound**. Debug-asserts may exist as defense-in-depth but **the criterion is
compiler enforcement**.

⛔ **NOT IN SCOPE:** performance work, new features, the divergent working
branch itself, version bumps, changelog, release. **A measured red beyond
this scope is REPORTED, not chased.**

---

## Coordinates — re-derived on this tree, confirmed at the briefed lines

The five `'static` returns over heap memory in the four families:

| # | Coordinate | Signature |
|---|---|---|
| 1 | `crates/beamr/src/term/boxed/accessors.rs:113` | `pub fn limbs(self) -> &'static [u64]` (BigInt) |
| 2 | `crates/beamr/src/term/boxed/accessors.rs:281` | `pub fn as_bytes(self) -> &'static [u8]` (ProcBin) |
| 3 | `crates/beamr/src/term/boxed/accessors.rs:340` | `pub fn as_bytes(self) -> &'static [u8]` (SubBinary) |
| 4 | `crates/beamr/src/term/boxed/accessors.rs:357` | `fn parent_bytes(parent: Term) -> Option<&'static [u8]>` — **THE KNOWN ANSWER** |
| 5 | `crates/beamr/src/term/shared_binary.rs:79` | `pub(crate) fn bytes_from_raw_word(raw: u64) -> &'static [u8]` |

**Also in-family, and the tie cannot complete without them** (see
`.fleet/plan.md` §2 Class B for the derivation):

- `crates/beamr/src/term/binary.rs:70` — `Binary::as_bytes` — **called
  directly by `parent_bytes`** at `accessors.rs:359`.
- `crates/beamr/src/term/binary_ref.rs:30` — `BinaryRef::as_bytes` — the
  public fan-in over all three binary families; what most callers use.

Nine further sites are **compiler-forced consumers** (plan §2 Class C) —
they cannot compile as `'static` once the above are tied, and that is the
no-laundering rule working for you. Three more (plan §2 Class D) manufacture
`'static` independently via `from_raw_parts` and are **out of scope —
report, do not chase**, unless the compiler proves one of them consumes a
tied accessor after all. **Follow the compiler, not the table.**

---

## R1 — THE DESIGN ARTIFACT, COMMITTED **BEFORE** IMPLEMENTATION

`docs/design/accessor-lifetimes.md`, in-repo, stating:

- **(a)** the chosen lifetime-tying mechanism **and the alternatives you
  rejected and why**;
- **(b)** the full inventory of the four accessor families' callers
  (**counted, per crate, jit included**);
- **(c)** the API-breakage assessment — which public signatures change and
  what that means for the next version (**report the fact; version decisions
  are NOT this flight's**);
- **(d)** how the GC generation/borrow interaction is made
  **UNREPRESENTABLE rather than checked-at-runtime**.

This artifact is committed **before any signature moves**. The judge checks
the ordering in the commit graph.

Terrain for (a) and (d) — plan §3/§4. The short of it: every allocating and
collecting path takes `&mut` (`heap.rs:285` `Heap::alloc(&mut self)`,
`gc/mod.rs:135` `gc::alloc(&mut Process)`, `:149` `ensure_space(&mut
Process)`, `:101`/`:118` the collectors, `process/gc.rs:43` `root_set(&mut
Process)`), so a slice tied to a **shared** borrow of `Heap`/`Process` makes
the collector's `&mut` unobtainable while it lives. That is
unrepresentability, not a check.

⚠️ **ONE MEASURED EXCEPTION, and it lands on T1 — plan V3.**
`gc/mod.rs:310` `release_all_refcounted_resources_in_heap(heap: &Heap)`
**frees** ProcBin `Arc` buffers through a **shared** `&Heap`, walking via
`Heap::visit_boxed_objects(&self, …)` (`process/heap.rs:452`). Its only
caller is `replay/driver.rs:34` inside `impl Drop for DecodedHeaps`
(`:31`-`:37`), where ownership closes the hole — dropping the `Vec<Heap>`
requires ownership, so no `&Heap` borrow into it can be live. **The strong
tie is still reachable.** But if you choose **T1 (tie to `&Heap`)**, R1(d)
must address this signature explicitly — it is a free that needs no `&mut
Heap` — or reject T1 for it. **T2 (tie to `&Process`) is untouched**: that
path holds no `Process` at all. Design input only; you do not modify
`release_*`, it is not one of the four families.

Candidate mechanisms T1–T5 and the rejected alternatives are laid out in plan
§4 — **you choose and you justify; the plan does not choose for you.**

### ⛔ THE WALL THAT MAKES A FAKE FIX POSSIBLE — read twice
A lifetime parameter appearing **only** in the return type or in a
`PhantomData` field, with **no witness in argument position**, is
unconstrained: caller inference picks whatever it needs, `'static` included.
That passes the `'static` sweep and **keeps the bug**. It is lifetime
laundering by inference. **The witness borrow must appear in an argument
position** (or in the constructor that produces the accessor). If your
design cannot state which argument constrains the returned lifetime, it is
not a fix.

---

## R1 NOTE — THE RED THAT MUST EXIST FIRST

**Before the fix**, a demonstration of the dangle: obtain a slice from one of
the four accessors, force a GC-triggering allocation while holding it,
observe the stale read or the miri failure.

**THIS VENUE HAS MIRI PROVISIONED** — verified executable on this tree:
`miri 0.1.0 (8925ea358a 2026-08-20)`. **Prefer it.** Whichever instrument you
use, **SAY WHICH**.

⚠️ Operational: miri is only reachable after `export
PATH="$HOME/.cargo/bin:$PATH"`. Without it the default PATH finds Arch system
rust 1.94.0 and `cargo +nightly miri` fails with *"no such command:
`+nightly`"* — which reads as "miri absent" and is wrong.
`.fleet/gate-entry.sh` already does this export; **do it in every shell.**

Committed **verbatim**: `docs/evidence/beamr-accessor-dangle-red.txt`.
(`docs/evidence/` does not exist yet — create it.)

**After the fix, the same program must FAIL TO COMPILE — capture the
compiler error as the green evidence: the bug is now a type error.**

If a true runtime repro is impractical for some accessor, **say so and show
the miri/borrow-level evidence instead — never skip the red silently.**

Precedent for the red's shape, already in the tree:
`crates/beamr/src/native/stdlib_stubs/gc_rooting_tests.rs` forces collections
mid-allocation with boxed terms live on a deliberately small heap
(`Process::new(1, 96)`, `:37`).

Note for `bytes_from_raw_word`: the `Arc<Vec<u8>>` buffer does not *move* on
GC, so a naive read says `'static` is harmless there. It is not — the GC
**releases** the Arc (`gc/mod.rs:282`, `:310`, `:321`), so the bound is the
ProcBin's liveness, which is the heap's.

---

## R2 — IMPLEMENTATION

The signatures change, **every caller compiles against the new bounds**, **no
transmutes or lifetime-laundering anywhere in the path** — *a lifetime fix
that launders internally has fixed the signature and kept the bug.*

**Every unsafe block this flight touches carries its safety comment
re-derived against the NEW lifetimes.** The judge reads them, not just counts
them. A safety comment that still argues from `'static` after the tie is a
failed re-derivation.

Banned in the fix path: `transmute` in any spelling, lifetime-widening
pointer round-trips, fresh `slice::from_raw_parts` that re-manufactures an
unbounded lifetime, and unconstrained lifetime parameters (see the wall
above).

---

## R3 — THE PROOF + REPORT

- The **caller inventory re-counted after implementation**. The authoritative
  instrument is the compiler's error list, not grep — grep counts are a
  pre-estimate only (`.limbs()` collides with `BigIntValue::limbs`,
  `.as_bytes()` with `str`/`String`/`SharedBinary`). Say which instrument
  produced the number.
- **ZERO remaining `'static` returns over heap memory in the four families —
  a sweep COMMAND WITH OUTPUT, not an assertion, with
  `accessors.rs:357` in the found-and-retired list.** A final sweep that does
  not list `:357` as found-and-retired **is a failed sweep.** Account for
  every surviving `'static` line in the sweep output — the two `&'static str`
  literals at `term/json.rs:22,:34` are error-message literals, not heap
  memory; say so rather than leaving residue unexplained. If you widen the
  sweep past `term/` (the workspace census is 60 `'static`-returning fns),
  two further sites are benign and **must not be "fixed"** — plan V6:
  `native/stdlib_stubs/encoding_bifs.rs:83` `base64_alphabet` (a genuine
  static constant, **in the same file as Class C `:203`**) and
  `jit/runtime_closure.rs:144` `runtime_cache` (JIT cache, not process heap).
- **The compile-fail evidence per accessor family.**
- **The API-breakage list.**
- **The flagged follow-on:** re-merging into the divergent working branch is
  the **OWNER's** work — **name the actual divergent branch by MEASUREMENT**
  (`git for-each-ref` with ahead/behind counts against main), **never by
  repeating the audit's stale shape.** Plan §10 carries the measurement as of
  planning time — `main` is an ancestor of **no** divergent branch, and the
  topical one is `origin/fix/0163-borrow-across-alloc` (401 behind / 15
  ahead, and its diff against main touches nothing under
  `crates/beamr/src/term/`). **Re-run it yourself at flight time; refs move.**
  ⚠️ **Exclude `refs/*/fleet/*` and `origin/HEAD` from the ancestor probe** —
  plan V5. `git merge-base --is-ancestor main <ref>` is now TRUE for four
  refs and **all four are this workflow's own** (this flight's base, and
  flight 1's rescued `origin/fleet/beamracc-flight1-flagged`). They did not
  exist when the plan was first measured. Report one of them as "the
  divergent working branch the owner must re-merge into" and you have made
  the stale-shape error from the opposite direction.
- **Every claim OBSERVED or REASONED with `file:line` or command output.**

---

## THE GATE

**The final gate is the repository's OWN ci-verdict harness (`gates.json`)
and it must run green.** Entry: `.fleet/gate-entry.sh` →
`./scripts/ci-verdict.sh gate-rc gates.json`. Nine legs: `fmt`, `clippy`,
`wasm32-check`, `wasm-tests`, `tests`, `blocking-call-in-native-bif`,
`clippy-all-features`, `tests-all-features`, `nostd-ratchet`.

**Never weaken, skip or silence a check, and never edit the gate entry.**

### ⛔ V1 — THE ENTRY DOES NOT RUN THE LEGS. IT ONLY GRADES THEM. Read this before your first gate run.

`ci-verdict.sh` is a **grader** (`:34` `Usage: ci-verdict.sh [GATE_RC_DIR]
[GATES_JSON]`). It reads `gate-rc/declared.count` and `gate-rc/<leg>.rc` /
`<leg>.log` that a **separate canon step** must already have written. Run
`.fleet/gate-entry.sh` on its own and you get, OBSERVED on the untouched tree:

```
no declared-leg count: the canon step did not reach its first leg
GATE_EXIT=1
```

That is **not a red gate** — it is a gate that never ran. The leg-running loop
lives in **`.github/workflows/ci.yml:68-112`**, not in `scripts/`. Run it
yourself, then invoke the entry:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p gate-rc
declared=$(jq -r '.legs | length' gates.json)
echo "$declared" > gate-rc/declared.count
for i in $(seq 0 $((declared - 1))); do
  name=$(jq -r ".legs[$i].name" gates.json)
  cmd=$(jq -r ".legs[$i].cmd" gates.json)
  ( eval "$cmd" ) > "gate-rc/${name}.log" 2>&1   # SUBSHELL + REDIRECT, never a pipe
  echo "$?" > "gate-rc/${name}.rc"
done
./.fleet/gate-entry.sh          # grades; DO NOT EDIT IT
```

Three properties are load-bearing, and `ci.yml:68-93` says so itself: **no
`set -e`** (every leg must run; the verdict comes from the recorded set), **a
subshell** around `eval` (so a `cd`-shaped leg cannot relocate later legs —
leg order must not affect the verdict), and **redirect, never pipe** (a pipe
hands `$?` to the downstream tool and records the wrong leg's rc).
`gate-rc/` is gitignored already (`.gitignore:21`), so this does not dirty
the tree. Takes ~4 min warm.

### ⛔ V2 — THE BASELINE GATE IS ALREADY RED HERE: FIVE OF NINE LEGS, ON THE UNTOUCHED TREE.

Flight 1 measured `cargo check` (`Finished dev profile`, **0 errors** — still
true, re-confirmed). It never ran the gate. Flight 3's planning seat did, on
the tree **byte-identical to beamr `main`** (`git diff --name-only
c55ac360..HEAD` = only `.fleet/*` and `.gitignore`):

| Leg | rc | Cause |
|---|---|---|
| `fmt` | **0** | — |
| `clippy` | **101** | 15 lint errors — `native/file_meta_bifs.rs` ×8 (`unnecessary_cast` `u32`→`u32`, `:251`-`:264`), `io/uring.rs` ×7 (`:10`,`:55`,`:63`,`:138`, +3) |
| `wasm32-check` | **0** | — |
| `wasm-tests` | **0** | 86 passed |
| `tests` | **101** | **1 test fails**: `tests/thread_inventory_distribution.rs:59` — `dist-send=1, net-kernel=0`. 2126 passed / 1 failed |
| `blocking-call-in-native-bif` | **0** | — |
| `clippy-all-features` | **101** | same lint family, **same count of 15** |
| `tests-all-features` | **101** | transient `rustc SIGBUS` first run; clean re-run → **the same one test** as `tests` |
| `nostd-ratchet` | **3** | `mktemp: too few X's in template 'nostd-ratchet'` — the instrument is **dead on this venue** |

**Five red legs, THREE root causes, none of them yours:** the pre-existing
clippy lints (legs 2+7), one deterministic test failure (legs 5+8), and a
`mktemp` incompatibility (leg 9). **Not one is in the four accessor families
or reachable from them.**

- **Leg 5 is not a flake.** Run 6 times, failed 6 times — 5× sandboxed, 1×
  unsandboxed. Not in `docs/KNOWN-INTERMITTENTS.md`, and that registry
  forecloses the shrug anyway: *"An entry here does not license dismissing a
  red."* Mechanism OBSERVED (`distribution/mod.rs:191-197` builds the
  net-kernel tokio runtime with `builder.build().ok()`, swallowing the error,
  so `worker_thread_names()` returns empty = `net-kernel=0`); root cause NOT
  established, and **out of scope either way**.
- **Leg 9 makes §7's trap #1 moot here.** The ratchet cannot measure the
  no-std tally, so you cannot lower `CEILING` even if your change legitimately
  improves it. The script's own REFUSE arm fires correctly — that is the
  harness *working*, refusing to green falsely. `gates.json` declares **no**
  `cannot_measure_rcs`, so rc=3 grades as a plain FAIL.
- **Leg 8: re-run on any SIGBUS before believing it.** It is a crash in the
  compiler (LLVM context teardown), not a diagnostic about this tree.

#### ⛔ YOUR GATE OBLIGATION IS DIFFERENTIAL. BINDING.

1. **Capture YOUR OWN baseline before the first edit** — run the V1 canon loop
   on the untouched tree and keep `gate-rc/`. Do not inherit the table above
   on faith; venues and toolchains move. This is the control for everything
   you claim afterwards.
2. **Add no red.** Every leg green at baseline is green at the end;
   `clippy`'s error set is **unchanged in content and count** (15); `tests`
   shows the **same one** failure and no other.
3. **Fix none of it.** `file_meta_bifs.rs`, `io/uring.rs`,
   `thread_inventory_distribution.rs`, `gate-nostd-ratchet.sh` — all out of
   scope, all *"a measured red beyond this scope is REPORTED, not chased."*
   **`#[allow]` nothing** (banned in any spelling). **Edit no gate script and
   no gate entry.**
4. **Report baseline and final side by side, per leg**, in R3. Claim "gate
   green" without a baseline and you are reporting what you cannot have
   measured; claim "gate red" without one and you will blame your own change
   for `file_meta_bifs.rs`.
5. **This is the measured-red wall's exact shape.** An honest differential —
   *"the four families are retired, the compiler enforces the bound, and the
   gate is red on the same five legs it was red on before I started"* — **is a
   success outcome**, even when the workflow's grammar renders it
   `gates_red/Failed`. **Do not manufacture a green.**

### The compile-fail evidence lives OUTSIDE the gate — with one exception the repo grants you
A program that must not compile cannot live in the gated tree. **Capture the
rustc output verbatim into `docs/evidence/` and delete the program** — **or**
use a trybuild-style compile-fail test **only if the repo already carries
that harness**. **NO new dependencies.**

**The repo does already carry that harness**: rustdoc ` ```compile_fail `
doctests, at `crates/beamr/src/process/mod.rs:146` and `:155`, used for
exactly this kind of negative type-level proof (that `Process` is neither
`Send` nor `Sync`). Doctests are enabled and run under the gate's `tests`
leg. So you may put the compile-fail proof in-gate as a `compile_fail`
doctest, **and** you still capture the verbatim rustc output into
`docs/evidence/`. Do not add `trybuild` or any other dependency.

⚠️ Pin the expected diagnostic — ` ```compile_fail,E0502 ` (or the applicable
E0499/E0505). A bare `compile_fail` passes when the snippet fails for *any*
reason, a typo included, which would be a green that proves nothing.
⚠️ Doctests reach only the **public** API. `parent_bytes` is private — prove
it through the public `SubBinary::as_bytes` path.

### Two gate legs that will bite this change specifically
1. **`nostd-ratchet` is strict in BOTH directions** — ⚠️ **but see V2: on this
   venue the ratchet CANNOT MEASURE AT ALL** (`mktemp` template
   incompatibility), so it is red at baseline and you cannot lower `CEILING`
   even if your change earns it. Keep the reasoning below for a venue where
   the instrument works; **do not "fix" the script here.**
   `scripts/gate-nostd-ratchet.sh:81` `CEILING=1072`; it fails if the tally
   exceeds it (`:120`) **and** if the tally is below it (`:127`), instructing
   `Set CEILING=$tally in this script, in the same commit`. Touching `term/`
   signatures will likely move the `--no-default-features` tally. A tally
   that goes **down** is legitimate — lower the ceiling in the same commit,
   as the script itself instructs. A tally that goes **up** is new no-std
   breakage: **fix it, never raise the ceiling** (`:81` "LOWER THIS, NEVER
   RAISE"). Count with rustc's own "due to N previous errors" line, not
   `grep -c '^error'` — the script's note records that off-by-one trap.
   Lowering the ceiling per the script's own instruction is **not** weakening
   a check; raising it would be.
2. **`clippy` and `clippy-all-features` run `-D warnings` across three
   feature configurations.** Lifetime parameters draw `needless_lifetimes`,
   `elidable_lifetime_names`, `extra_unused_lifetimes`. **Fix the code. Never
   `#[allow]` the lint** — suppressions are banned in any spelling.

---

## HARD WALLS, BINDING

- **No lint suppressions in any spelling.**
- **No ignore attributes on tests** — runtime env-gates only.
- **No file over 500 lines of code** — operationally, for a repo where
  **134 files already exceed it on untouched `main`** (plan V4): **do not
  create a new over-500 file, and do not grow a file you touch past 500.**
  ⚠️ `crates/beamr/src/term/boxed/accessors.rs` is at **472 lines** — 28 from
  the wall, and it is the most-edited file of this flight. Lifetime
  parameters plus re-derived safety comments will exceed 28 lines. **Plan the
  module split up front**, do not discover the wall at the end. Files already
  over the wall that you must not grow: `term/compare/mod.rs` 713,
  `process/mod.rs` 1361, `native/stdlib_stubs/gc_rooting_tests.rs` 552 —
  pre-existing, report as measured, do not chase. (`native/context/mod.rs`
  is 1815 and sits in the T4 blast radius — same rule.)
- **Never `.unwrap()`/`.expect()` in library code.** The current
  `SubBinary::as_bytes` uses `unwrap_or`/`checked_add` correctly
  (`accessors.rs:341-344`) — keep that discipline through the rewrite.
- **`unsafe` is expected in a VM's term internals**, but every touched
  `unsafe` block carries its **re-derived** safety comment.
- **No new dependencies, no version bumps, no changelog, no release prep.**
- ⛔ **If you run any long check in the background, NEVER one blocking sleep
  sized to the expected duration — poll at a stated interval under a stated
  deadline and report expiry as a TIMEOUT, a distinct outcome.**

---

## THE MEASURED-RED WALL

**If the design pass concludes the lifetime tie cannot be done without
restructuring beyond this scope, that CONCLUSION with its evidence IS the
deliverable — committed, reported, no heroics.**

Consequence acknowledged: under the workflow's grammar an honest
measured-red termination arrives as **gates_red/Failed** — **THE FAILURE
GRAMMAR IS THE SUCCESS OUTCOME** in that branch. The tree, the committed red,
and the report are the deliverables either way. Do not manufacture a green.

---

## LANE BOUNDARY, BINDING

This flight produces a **CANDIDATE result branch**. **Nothing merges to beamr
`main` at any seat in this workflow** — the landing is ratified by the repo
owner's seat.

---

## THE REPORT PATH IS A WIRE CONTRACT

**`.fleet/reports/accessor.md`** — byte-exact, enforced mechanically after
fan-in. Confirm with `git ls-tree HEAD -- .fleet/reports/` before declaring
done; **the listing showing the blob at the contracted path is the receipt.**
