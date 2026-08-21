# LEG BRIEF — accessor lifetimes (builder seat)

Flight `beamracc-lifetimes-flight1`. Base: `fleet/accessor-lifetimes-base`
(`main` `c55ac360` + .fleet documents). **Read `.fleet/BRIEF.md` whole
before you touch anything** — it is the brief of record and it governs.
`.fleet/plan.md` is the terrain map: coordinates, classification, gate
traps, all re-derived on this tree. Read it second.

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
unrepresentability, not a check. Candidate mechanisms T1–T5 and the
rejected alternatives are laid out in plan §4 — **you choose and you justify;
the plan does not choose for you.**

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
  memory; say so rather than leaving residue unexplained.
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
- **Every claim OBSERVED or REASONED with `file:line` or command output.**

---

## THE GATE

**The final gate is the repository's OWN ci-verdict harness (`gates.json`)
and it must run green.** Entry: `.fleet/gate-entry.sh` →
`./scripts/ci-verdict.sh gate-rc gates.json`. Nine legs: `fmt`, `clippy`,
`wasm32-check`, `wasm-tests`, `tests`, `blocking-call-in-native-bif`,
`clippy-all-features`, `tests-all-features`, `nostd-ratchet`.

**Never weaken, skip or silence a check, and never edit the gate entry.**

Baseline OBSERVED on this tree before any edit: `cargo check --workspace
--features beamr/encode` → `Finished dev profile in 18.37s`, **0 errors**.

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
1. **`nostd-ratchet` is strict in BOTH directions.**
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
- **No file over 500 lines of code.**
  ⚠️ `crates/beamr/src/term/boxed/accessors.rs` is at **472 lines** — 28 from
  the wall, and it is the most-edited file of this flight. Lifetime
  parameters plus re-derived safety comments will exceed 28 lines. **Plan the
  module split up front**, do not discover the wall at the end. Files already
  over the wall that you must not grow: `term/compare/mod.rs` 713,
  `process/mod.rs` 1361, `native/stdlib_stubs/gc_rooting_tests.rs` 552 —
  pre-existing, report as measured, do not chase.
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
