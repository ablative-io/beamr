# B-144 — evidence base, and a re-sizing

*Authored 2026-08-17 at `dfbfc2a`, from the #219 feature-gate sweep and its two
landed lanes. Ruled by Waffles at seq=40-ack: one authoring pass covering F1–F6,
the (a) result with both corrections, and the portability finding with its
worklist.*

**As authored, this document deliberately did not change `B-144.json`.** Waffles
then ruled at seq=41-ack that R2 and R3 be marked satisfied citing this file as
the evidence, and that edit has been made — see *Status of the routed items*
below. Everything measured here still stands on its own; the brief now agrees
with it.

⚠️ **THIS FILE WAS RENAMED, AND THE OLD NAME WAS A HAZARD.** It was first
committed as `B-144.md`, which is the **render target** of
`.meridian/design-system/scripts/render-brief.py B-144.json B-144.md` — the
generated companion every other `B-NNN.md` in this directory is. Rendering B-144
would have silently overwritten this evidence document, including the citation
the brief now depends on. Renamed to `B-144-EVIDENCE.md`, matching the existing
`WPORT-1-EVIDENCE.md` / `B-029-SUMMARY.md` precedent. ⭐ **A FILENAME CAN BE A
GENERATED-OUTPUT PATH; WRITING TO ONE IS A DELETION WITH A DELAY ON IT.**

---

## ⭐ HEADLINE: B-144 IS LARGELY SATISFIED, AND ITS REMAINING PIECE IS ~50× SMALLER THAN THE OBVIOUS NUMBER SUGGESTS

| req | claim | status at `dfbfc2a` |
|---|---|---|
| **R1** no_std compatibility audit | gate std-dependent code behind features | ⚠️ **OUTSTANDING — but 20 errors, not 1039** |
| **R2** single-threaded scheduler for WASM | create `crates/beamr/src/scheduler/wasm.rs` | ✅ **SATISFIED** — verified against acceptance text |
| **R3** WASM host bindings | create beamr-wasm, export 4 fns | ✅ **SATISFIED** — verified against acceptance text |

R2 and R3 were discharged by the WPORT arc; the brief was never updated.
**Marked satisfied in `B-144.json` on Waffles' seq=41-ack ruling.**

### ⚠️ AND THE SAME LAW APPLIED AGAIN, TO MY OWN CLAIM

This table first read “**SATISFIED** — file exists” for R2 and “crate exists,
all four present” for R3. **File existence is not the acceptance criterion.** R2
names five things and R3 names four exports plus a serialization form, so before
writing SATISFIED into the durable brief record I checked those, not the paths:

| req | acceptance names | measured at `4b68e74` |
|---|---|---|
| R2 | `WasmScheduler` type | `scheduler/wasm.rs:119` |
| R2 | `run_until_idle()` one scheduling round | `:517`, returns `WasmRunSummary` |
| R2 | yield via reduction counting | reduction-bounded slices, `MAX_SLICES_PER_DRAIN` |
| R2 | host calls it from its own event loop | `beamr-wasm/src/lib.rs:792` |
| R2 | test executes a function to completion | `scheduler/wasm_tests.rs` |
| R3 | `#[wasm_bindgen]` exports | `beamr-wasm/src/lib.rs:49` and following |
| R3 | `create_vm()` | `:50` |
| R3 | `load_module(bytes)` | `:208` |
| R3 | `spawn(m, f, a)` | `:293`, args carried as JSON |
| R3 | `run_step()` | `:309` |

Standing witness for both: canon gate legs `wasm32-check` and `wasm-tests`, green
in every battery this arc. The verdict did not change — but it was resting on a
weaker check than the one the requirement asked for, which is exactly the trap
this document opens by describing.

### ⛔ THE TRAP THAT WOULD HAVE MIS-SIZED R1 BY FIFTY TIMES

There are **two different no-default-features checks** and they are not the same
problem. R1's acceptance names the second one:

| command | errors | why |
|---|---|---|
| `cargo check -p beamr --no-default-features` (**host**) | **1039** | on a non-wasm target `lib.rs:6`'s `all(not(std), not(wasm32))` is TRUE, so the crate really is `no_std` and every `std::` path breaks |
| `cargo check --target wasm32-unknown-unknown -p beamr --no-default-features` (**R1's actual criterion**) | **20** | on wasm32 that same predicate is FALSE, so `no_std` is never applied and std remains available |

The 1039 is real and is what the `nostd-ratchet` gate watches. **It is not R1's
number.** Quoting it into R1 would make a 20-error job look like a rewrite.

⭐ LAW APPLIED: **GATE THE COMMAND THE REQUIREMENT ACTUALLY NAMES.** I had the
1039 in hand and it was tempting to carry it straight in. R1's acceptance text
names the wasm32 target explicitly, so I measured *that* command, and it
disagreed with the number I already had by a factor of ~52.

### R1's twenty errors, at the bytes — three root causes

| count | unresolved import |
|---|---|
| 7 | `crate::timer` |
| 5 | `crate::replay` |
| 1 | `crossbeam_queue` |

13× E0432, 7× E0433. `timer` and `replay` are feature-gated modules referenced
from code that is not equivalently gated; `crossbeam_queue` is the `cooperative`
dependency referenced without its feature. Three fixes, not three hundred.

⚠️ **TWO CORRECTIONS TO THE TABLE ABOVE, both measured at `a4e2328`.**

**(a) Those three rows sum to 13, not 20** — they count the E0432 *imports* only.
The seven E0433s were counted in the prose and never attributed. Whole-20
attribution: `crate::timer` 12, `crate::replay` 7, `crossbeam_queue` 1.

**(b) THERE IS A FOURTH ROOT CAUSE, and it was invisible here.** Clearing the
import layer exposed 6× `E0277: ExecError: From<ReplayMismatch> is not satisfied`
from a *separately gated* `From` impl at `error.rs:447` — masked while the import
errors stood. The count went 20 → 7 → 0 over four edits.

⭐ **ERRORS REVEAL IN LAYERS; AN ERROR COUNT IS A LOWER BOUND ON THE WORK, NEVER A
DESCRIPTION OF IT.** A root-cause count is provisional until the build is green.
It resolved cheaply here — the same reasoning could as easily have hidden 40.

---

## ⛔ AMENDMENT — R1's SPEC SENTENCE CARRIES A FALSE PREMISE. RULED BY WAFFLES, seq=42-ack.

R1's spec in `B-144.json` reads:

> ~~"Identify and **gate** std-dependent code behind feature flags."~~

**Struck, not reworded.** That sentence was written when the crate was believed
`no_std`-capable — the *same* false premise this arc already struck from
`docs/stack-review/beamr-architecture.md` as **F6**. On `wasm32-unknown-unknown`,
`lib.rs:6`'s `all(not(std), not(wasm32))` predicate is **FALSE**, so `no_std` is
never applied and `std` is available. Gating std-dependent code is therefore not
what makes R1's named build compile, and never was.

**The true requirement**, as R1's own acceptance command always named it: *the
`wasm32-unknown-unknown --no-default-features` build compiles, with module
availability consistent across the configuration.*

⭐ **THE RULING'S REASONING, WORTH KEEPING: honouring a spec sentence whose premise
has already been refuted is not honouring the spec — it is obeying a fossil.**
A requirement has two texts, its prose and its acceptance criterion, and when they
disagree it is the *criterion* that survived contact with the compiler. This is
the same law as GATE THE COMMAND THE REQUIREMENT ACTUALLY NAMES, arriving from the
other direction: there, the prose under-described the work by ~52×; here, the
prose points at the wrong work entirely.

Consequence: the fix un-gates `timer` and `replay` rather than gating their 20
reference sites. See `gate-logs/B-144-R1/` for the measured fork, the ratchet
population change, and the spot-proof that bought the "revealed not created"
reading.

---

## THE SWEEP'S FINDINGS (F1–F6)

### F1 · `--no-default-features` was INERT under `cargo test` — **FIXED**, lane (a), `9c3be75`

`crates/beamr/Cargo.toml` dev-depended on beamr **itself** without
`default-features = false`. The flag binds the package named on the command
line; that dev edge was a separate graph edge carrying `default = true`, and
cargo unifies features across edges — so the defaults came straight back in.

Confirmed at two instruments before the fix, and at the decisive one after:
`cargo tree -p beamr --no-default-features --features cooperative,json
-e features` now returns exactly `beamr feature "test-support"`, where it
previously returned the full default set.

### F2 · the waiver lived in PROSE, not in a gate — **FIXED**, lane (b), `726607b`

No `gates.json` leg ran the no-std check. The waiver was recorded honestly in
CHANGELOG.md and `docs/DIST-CONTROL-WIRE-SPEC.md` and was unwatchable by
construction. Now leg 9, `nostd-ratchet`.

### F3 · legs 5 and 8 build `cooperative` AND `threads` together — **OPEN, routed to Tom's docket**

`beamr-wasm` is a workspace member depending on beamr with
`default-features = false, features = ["cooperative","json"]`; under `--workspace`
those unify **into the host build**. Canon's host suite therefore runs a hybrid
configuration nobody ships. Control: `-p beamr` alone yields the same feature
list minus `cooperative`, so the workspace edge causes it, not the query.

### F4 · `--features std` alone is red — ~~21 errors~~ **20**, and it is the SAME FAILURE, not merely the same shape

~~"21 errors, effectively the same three root causes as R1's twenty."~~

**Both corrections measured at `a4e2328`.** rustc's own tally is **20**; a naive
`grep -c '^error'` on the same log returns 21, because the trailing
`error: could not compile …` summary is itself an `error` line — the exact
off-by-one the `nostd-ratchet` leg's counting rule exists to prevent, sitting in
the document that states the rule. ⭐ **A COUNTING RULE DOES NOT RETROACTIVELY FIX
THE NUMBERS ALREADY ON THE PAGE.**

And "same shape" understated it. Comparing full `(code, message, file:line:col)`
triples parsed from `--message-format=json`, the two commands' error sets are
**IDENTICAL** — 20 == 20, zero unique to either. Fixing R1 fixes F4 by identity.
No separate F4 work exists.

### F5 · the browser feature set IS target-portable at the LIBRARY level

`cargo check -p beamr --no-default-features --features cooperative,json` → **rc 0**
on the host. The library compiles. (Its *tests* do not — see the portability
section below; these are different claims and must not be merged.)

### F6 · a false docs claim — **FIXED**, lane (c), `726607b`

`docs/stack-review/beamr-architecture.md` called beamr "`no_std`-capable". It was
~1000 errors red at the v0.12.0 that document snapshots.

---

## THE RATCHET, AND WHY THE NUMBER MOVED AT ALL

`cargo check -p beamr --no-default-features` was **1019** errors at `ec5d7f8`
(2026-07-07) and **1039** at `d4a82e5` (2026-08-16). +20 across five minor
versions, unremarked.

⚠️ **COUNTING RULE — this has now caught two seats.** Use rustc's own
`due to N previous errors`. Do *not* count `^error` lines: the trailing
`error: could not compile …` summary line is itself an `error` line. The
historical record's **1020** and my own first reading of **1040** are both that
same off-by-one against 1019/1039. **Neither was wrong about the world.** State
this reconciliation wherever both numbers appear, or the next reader concludes a
seat miscounted and the accusation outlives the correction.

The original author is **not** contradicted: "byte-count-identical from baseline
`ec5d7f8` through this release" was true for the 0.13.0 series. What lapsed is
the five minor versions after it.

⭐ **THE CLASS: a waived gate with no ratchet, plus a harness structurally unable
to select the config, is debt that grows invisibly.** Neither half hides it
alone — the prose waiver was honest and explicit, the harness gap looked like
nothing. The debt grew in the **seam** between them.

---

## LANE (a) — THE RESULT, WITH BOTH CORRECTIONS CARRIED

### Correction 1 — a prediction I had already stated, corrected before the run

I told the lead to expect the canon axes to move on (a). Measurement said
otherwise **before** the edit: `beamr-cli` depends on beamr *with* defaults, and
beamr is itself a workspace member so `--workspace` enables its defaults
directly. Defaults reach beamr by two other paths in every canon leg. The canon
feature set is byte-identical before and after. Battery axes came back exactly
as re-predicted: 2/86/0/0 · 76/2150/0/0 · 76/2160/0/0.

### Correction 2 — an erratum against commit `9c3be75`'s own message

That message says cranelift-jit, tokio, mio, num_cpus and zstd are **all** absent
after the fix. Accurate:

| crate | gone? | why |
|---|---|---|
| cranelift-jit | ✅ | via beamr's `jit` default |
| mio | ✅ | via `readiness` |
| num_cpus | ✅ | via `threads` |
| tokio | ❌ | arrives via `opentelemetry_sdk`, a separately declared dev-dep — **no beamr feature involved** |
| zstd | ❌ | still compiled; `cargo tree -i zstd` prints "nothing to print" ⇒ **provenance UNRESOLVED, and not guessed** |

⛔ **The commit is deliberately NOT amended.** The battery ran at pin `9c3be75`;
amending changes the sha and orphans the evidence measured against it. *The bytes
that ran are the bytes that ship* outranks a tidy message.

⭐ **How it was caught, and the law it earns:** the probe's compile list held
crates the `cargo tree` check had reported absent. Two of my own instruments
disagreed, so at least one measurement did not mean what I had said it meant.
**A CRATE MISSING FROM A FILTERED VIEW IS NOT A CRATE MISSING FROM THE BUILD.**
Sits beside the false-zero external-pager trap.

Conflating "crate present in the graph" with "feature enabled" is the error
itself; the lane's real claim is about *features*, and that claim is intact.

---

## ⛔ THE PORTABILITY FINDING — beamr's test suite has never compiled in the configuration it ships to browsers

**The single biggest thing #219 produced.** Lane (a) turned an unmeasurable
configuration into a measurable one; the first measurement is a compile failure.

`cargo test -p beamr --no-default-features --features cooperative,json` → **rc 101**

**Seven integration targets fail to COMPILE**, on **15 unresolved imports**:

| count | unresolved |
|---|---|
| 4 | `beamr::scheduler::{Scheduler, SchedulerConfig}` |
| 3 | `beamr::scheduler::{NativeBifs, Scheduler}` |
| 3 | `beamr::native::gate3_bifs` |
| 1 | `beamr::scheduler::dirty` |
| 1 | `beamr::native::meridian_ffi` |
| 1 | `beamr::native::gleam_ffi` |

### Worklist — the seven targets

| target | file |
|---|---|
| `mfa_provenance_e2e` | `crates/beamr/tests/mfa_provenance_e2e.rs` |
| `gleam_gate_e2e` | `crates/beamr/tests/gleam_gate_e2e.rs` |
| `is_function_bif` | `crates/beamr/tests/is_function_bif.rs` |
| `suspend_result_binary` | `crates/beamr/tests/suspend_result_binary.rs` |
| `composition_report` | `crates/beamr/tests/composition_report.rs` |
| `dirty_scheduler` | `crates/beamr/tests/dirty_scheduler.rs` |
| `supervision_integration` | `crates/beamr/tests/supervision_integration.rs` |

Plus one error inside `crates/beamr/src/scheduler/mod.rs` itself.

### What this is, and is not

- **NOT a regression.** The configuration was unbuildable before lane (a). Nothing
  broke; it became *askable*, and the answer is no.
- **NOT a gate.** The battery at `9c3be75` is 9/9 with all axes exact. This probe
  gates nothing.
- **NOT the same claim as F5.** The *library* compiles in this configuration
  (F5, rc 0). The *tests* do not. Merging those two would be false in both
  directions.
- **IT IS** the sweep's candidate finding 2, sharpened from a suspicion about
  test reach into a compile failure with named targets and named imports.

### ⛔ THE WORK IS NOT STARTED AND DOES NOT START WITHOUT A SEPARATE WORD

Ruled at seq=40-ack. Making the suite compile without `threads` is a **design
conversation about what the browser-config suite should even assert** — feature-
gate the seven targets, or provide cooperative equivalents, and those answer
different questions. Routes through Tom's docket alongside F3.

---

## STATUS OF THE ROUTED ITEMS

1. ✅ **DONE** — R2 and R3 marked `"status": "satisfied"` in `B-144.json`, each
   citing this file, on Waffles' seq=41-ack ruling.
2. ✅ **DONE** — R1 marked `"status": "outstanding"` and re-sized in place against
   its own named command: 20 errors, three root causes (`crate::timer`,
   `crate::replay`, `crossbeam_queue`). **R1 is now the brief's only open
   requirement.** F4's 21 fold in; same shape. GO given as the next lane.
3. **F3** — canon's host suite runs a cooperative+threads hybrid nobody ships.
   Tom's docket.
4. **Test-suite portability** — the seven targets above. Tom's docket, design
   conversation first.
5. The **host `no_std`** path (1039) is a separate and much larger question from
   R1, and nothing in this document proposes taking it on. The ratchet holds the
   number; it does not commit anyone to driving it to zero.

## ⚠️ DISCLOSURE — THE MARKING IS INVISIBLE TO THE RENDERER

`status` is read by **neither** `render-brief.py` nor `render-cluster.py`. A
rendered `B-144.md` would therefore still show three requirements with no
satisfaction marking at all. The JSON is the record; the rendered view is not.
Fifteen briefs already carry a top-level `status` (all `"approved"`, a
brief-lifecycle field) and it is equally unrendered, so this is a pre-existing
estate-wide gap, not one this lane introduced — **and per-requirement `status`
is a field no brief in the corpus carried before now.** Teaching the shared
renderer to show it would change output for every cluster, so it is disclosed
and routed rather than taken here.

## Provenance

Lanes: `726607b` (ratchet + doc) · `a2ffe61` (evidence) · `9c3be75` (manifest) ·
`dfbfc2a` (evidence) · `4b68e74` (this document) · this commit (brief marking +
rename). Full detail in `gate-logs/219/` and `gate-logs/219-a/`.
