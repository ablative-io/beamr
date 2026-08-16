# AR-1 FIX LANE — RESULTS

Artemis Peach, task #111. Design: `gate-logs/111/DESIGN.md`. Base:
`origin/main 8d64fd3`.

---

# PHASE 1 — `TermAccumulator`

Additive. No production call site changes; no sink signature changes; no site
migrated yet. New file `crates/beamr/src/native/context/accumulator.rs` plus two
one-line module/re-export edits.

## What it is

The rooted term accumulator: elements live in the process native root stack, so
a collection during the accumulation traces and forwards them. Full rationale in
`DESIGN.md` §1; the one fact that shapes everything is that **the sinks already
root their own arguments**, so the defect lives entirely in the loop that runs
before the sink is called.

## THE TWO-ARM CONTROL — and it is the point of the phase

`accumulator_survives_collections_that_break_a_bare_vec` runs 27 cells (3
lengths × 9 margins). Each cell runs **both** arms against the same heap, same
inputs, same pre-fill:

| arm | carrier | measured |
|---|---|---|
| synthetic positive | bare `Vec<Term>` — the pre-fix shape, written on purpose | **17 corrupt · 10 clean · 0 refused** |
| remedy | `TermAccumulator` | **0 corrupt · 27 clean** |

Both directions are asserted. The corrupt count is what makes the remedy arm's
green mean something; the clean count is what shows the reader is not simply
broken.

⭐ **This arm exists because inverting a probe kills its own control.** Every
row-4 probe asserts `red > 0` to prove it applied pressure. After the fix that
assertion cannot hold at the production site — so each inverted probe needs a
positive that survives the remedy, or its green is indistinguishable from a dead
instrument. Same law as R10's control fixtures, one level down. `DESIGN.md` §3.

## ⛔ THREE INSTRUMENT ERRORS OF MINE, CAUGHT IN THIS PHASE

### 1. The first control counted REFUSALS as corruption

First sweep reported a live positive control on 6 cells. Reading the printed
table: every one of them said `alloc_list refused` — the allocator correctly
declining a request a 4096-word heap could not serve. **Zero were corruption.**
A refusal is evidence of nothing about rooting.

Repaired by making the distinction **structural** rather than a string test: a
`Cell` enum with `Clean` / `Refused` / `Corrupt(reason)` variants, so no future
counter can merge them. (The row-4 probes discriminate the same two states with
`v.contains("returned an error term")`; the type is the better instrument.)

⭐ **A COUNTER OVER A `Result` COUNTS FAILURES, NOT THE FAILURE YOU MEAN.**
Caught only because the cells were printed. A bare pass/fail would have shipped.

### 2. The pinned pair was a HAND-COUNT off a printed table

Pinned `(20, 7)`; the next run failed at `(17, 10)`. Nothing had changed but
`cargo fmt`. The program was computing `corrupt_control` and `clean_control`
three lines above the assertion, and I typed my own tally of the table instead
of reading them — **so the assertion tested my arithmetic, not the instrument.**
Measured stable at `(17, 10)` over five runs, and again under the full parallel
lib suite. 17 + 10 + 0 = 27, and the ten clean cells are accounted for by name:
nine margin-0/1/2 rows where the pre-fill gives up immediately (achieved 4090 of
a 4096-word heap — no pressure applied at all), plus len 20 / margin 128, where
the accumulation fits inside the margin left free.

### 3. ⭐⭐ THE WORKTREE THAT WAS NEVER READ

`shape_hunt.py:207` walks `pathlib.Path("crates")` — **relative to the caller's
cwd, not to the script.** I checked out `8d64fd3` and `b77c21a` into a scratch
worktree, ran the script *by its path inside that worktree*, and read the result
as that commit's census. All three runs walked my own working tree. I paid for
two checkouts and measured the same files three times.

**And it produced plausible confirming output**: 40/27/13 at every commit, which
reads as "census stable across the lane" and actually meant "one file read three
times." It also appeared to contradict my own recorded 39/27/12 baseline, which
would have made me correct a record that was right.

Re-run with `cd` into the worktree:

| commit | totals |
|---|---|
| `b77c21a` (row-1 landing) | **39 raw · 27 production · 12 cfg(test)** — matches the record exactly |
| `8d64fd3` (row-4 landing, this lane's base) | **40 · 27 · 13** |
| this tree (phase 1 applied) | **40 · 27 · 13** — phase 1 adds ZERO |

⭐ Same family as #110's path-vs-program matcher: **I pointed the instrument at a
path I intended and it read a path it resolved.** Directing an instrument by
argument does not direct it by working directory, and an instrument that takes
its population from the cwd is steered by where it is *run*, not by what it is
*given*.

## THE +1 IS #110's, AND IT IS NAMED

`39/27/12 → 40/27/13` happened at the **row-4 landing**, not here. The added hit
is `crates/beamr/src/native/stdlib_stubs/string_bifs.rs:405`:

```rust
let joined = (0..parts).map(part_of).collect::<Vec<_>>().join("|");
```

— inside the `ar1_row4_site14_tests` probe module. It is a `Vec<String>`, not a
`Vec<Term>`: a **false positive of a deliberately syntactic hunt**, correctly
classified `cfg(test)`, and **production stays 27.**

⚠️ Recorded because row 2 exists to catch exactly this: a census that moves while
the lane that moved it says nothing. #110's battery did not re-run the hunt —
the hunt is not a canon gate leg — so the drift landed unremarked. **Not a
defect; an unclosed accounting line, closed here.**

## Shape-hunt controls

`ALL 5 CLASS CONTROLS PASS` — every class shown, in the same run, able to
produce a presence. `population walked: 349 files`. The accumulator module is
**invisible to the hunt** (`shape_hunt.py:210` skips `src/native/context/`),
which is correct — it is the remedy, not a defect — but it does mean the hunt
cannot police the remedy's own bytes. Recorded, not repaired.

## Legs run

- `cargo fmt --all` — applied.
- `cargo clippy -p beamr --all-targets --features encode -- -D warnings` — clean.
  ⭐ Run **before** the tests, not after: my own law from #110, *running a probe
  is not running the gate that will grade it*, and this lane's probes get graded
  by clippy too.
- `cargo test -p beamr --features encode --lib` — **1838 passed, 0 failed, 0
  ignored** (1835 + 3 new).

Full canon battery is owed at the phase's landing, not per commit.

---

# PHASE 2 — PER-SITE MIGRATION

Per-site commits, probes INVERTED not deleted, ledger disposition moving with
the inversion commit as receipt. Clippy before tests, every time.

| site | function | commit | disposition | census delta |
|---|---|---|---|---|
| 3 | `all_loaded` | `b2b0dbc` | STRUCTURALLY-ELIMINATED | production S3d 3 → 2 |
| 14 | `bif_split` | `43be80e` | STRUCTURALLY-ELIMINATED | **ZERO — see below** |

Both reproduce the same shape: the carrier is gone, not rooted-in-place, and
the replacement construct is named in the ledger at a `file:line` the checker
verifies at the bytes. ⭐ The checker has teeth — pointing site 14's
`replacement_site` at a wrong line was REFUSED, rc 2, *"the claim is refuted at
the bytes."* A checker that has never been shown to refuse is not a checker.

## ⛔ THE CENSUS CANNOT WITNESS THIS LANE'S COMMONEST SITE

Site 14's shape-hunt delta is **zero in both directions**: 40 raw / 26
production / 14 cfg(test) at the parent `b2b0dbc` AND at `43be80e`. Not a null
result — an instrument fact.

**`shape_hunt.py` has no class for a plain `Vec<Term>` push loop.** Its five
classes are `.map(..).collect()` (S3a), `match`/`if` bind (S3b), literal (S3c),
reassignment (S3d), and `Vec`-of-**tuples** push (S3e — its guard requires a
tuple-literal `var.push((`). The single most canonical AR-1 carrier —

```rust
let mut terms = Vec::with_capacity(n);
for part in parts { terms.push(context.alloc_binary(part)?); }
context.alloc_list(&terms)
```

— matches none of them. Site 14 was invisible before the fix and its replica is
invisible after.

**Established by positive control on the real instrument, not by reading the
regex.** A throwaway file carrying BOTH a plain push loop and an S3d
reassignment, dropped into a scratch worktree at the parent: total 40 → 41,
the added hit being the S3d meta-control at `:15`, and **zero** for the push
loop. The file was therefore demonstrably walked. ⭐ Reading a regex tells you
what you think it matches; running it against a known positive tells you what
it does.

⇒ **Sites fixed in this class have no machine-checkable census half.** The
two-arm probe is the whole of their evidence, and that is written into each
ledger row rather than left for a reader to infer from a zero. Sites 2 and 5
are the same shape and will behave the same way — *predicted here, before they
are done.*

⚠️ The hunt's 5 class controls PASS on every run, so it looks healthy while
being blind to this. A green control set certifies the classes it has, not the
classes it lacks.

## 🔴 AND THE ALARM I RAISED OFF IT DID NOT SURVIVE — RETRACTED

The blind spot led me somewhere much bigger, and wrongly. A syntactic probe for
the uncovered class returned **29 raw / 25 production** candidates, ~16 of them
in files the ledger never names (`process_info_bifs`, `json_bifs`, `ets_bifs`,
`system_info_bifs`, `inet_bifs`, `etf/decode`). Read one way that is *"the AR-1
population is undercounted, and 17 shipped in a security advisory."*

⛔ **THE COUNTERWEIGHT APPLIED: a claim large enough to alarm gets re-measured
with a different instrument BEFORE it is sent.** It was not sent. It was
measured, and it died:

1. **`ordering_detector.py:264` grades the LEDGER** — `population: {len(sites)}
   ledger carriers + controls`. It never discovered anything; it cannot be the
   source of the 17.
2. **`sink_census.py` is the discovery instrument, and it is SINK-DERIVED, not
   carrier-derived.** It walks every call to the collecting sinks and
   classifies the argument literal-vs-variable. A push loop still ends at
   `context.alloc_list(&terms)`, so it is caught there however the carrier is
   spelled. **75 variable-shaped production sink rows → 17 crossings**, and
   every file I flagged is in its 250 rows.
3. **The funnel is documented and adversarial.** `AR-1-GROUND-PACK.md` carries
   *"Ruled NOT crossings (checked, not assumed)"* — including
   `encoding_bifs.rs:34` **"not Terms"**, which is precisely the false positive
   I had predicted from the `Vec<u8>` byte buffer, and `closures.rs:423,482`
   **"no allocating call in the loop."**

⇒ **The population is not undercounted by my mechanism, and I retract the
suggestion that it is.** What survives is the narrow, true version: *the CENSUS
cannot witness push-loop sites*, which bears on row-2 accounting for this lane
and on nothing else.

⭐ The population's real blind spot is already declared and is NOT mine: only
*direct* calls to the 37 named collecting primitives are recognised, so a
crossing whose only collecting call is one level of indirection is invisible
(`AR-1-GROUND-PACK.md` §6). Already recorded by the ground pack's author; I add
nothing to it and do not re-raise it as new.

⭐⭐ **THE GENERAL FORM, and it is the one worth keeping: A BLIND SPOT IN ONE
INSTRUMENT IS NOT EVIDENCE OF A GAP IN THE FINDING THAT INSTRUMENT DID NOT
PRODUCE.** I found a real hole in the census and reflexively read it as a hole
in the population — but the census never sized the population. Before inheriting
an instrument's defect into a conclusion, check that the conclusion was ever
drawn from that instrument.

## Site 14 — the two-arm evidence

- **Positive control asserted FIRST**, and it corrupts:
  `part 0: contents "p000000000256" != "p000000000000"`. A **contents
  mismatch, not a refusal** — the exact ambiguity that fooled the phase-1
  control, now discriminated by construction.
- **Negative control** at 250 parts pins the reader.
- **One shared `read_back`** across both arms, so neither is graded by the
  softer instrument. (Extracting it is what the phase-1 hand-off left undone;
  the tree did not compile until it was.)
- **Red-at-parent by running**, production hunk reverted alone: failed at the
  CLAIM (`string_bifs.rs:529`, *head is not a binary — carrier went stale*)
  with both controls green above it. The failure landed on the right line.

⚠️ **DISCLOSED, MECHANISM NOT CLAIMED:** 350 parts @ heap 1024 was REFUSED
pre-fix and is clean post-fix. Real behaviour change, stated rather than
absorbed. The probe's old comment describing the refusal is retained solely as
the `f993280` record and marked false of the shipped body.

## Legs — site 14

- `cargo fmt --all` · `cargo clippy -p beamr --all-targets --features encode --
  -D warnings` clean · `cargo test -p beamr --features encode --lib` **1838
  passed, 0 failed, 0 ignored** (unchanged — the probe was replaced, not added).
- `ledger_check.py` rc 0, with the `structural-evidence` check shown to refuse.

---

## Sites 5, 2, 7, 8+9, 1 — and the replica-fidelity check

Each site's probe was INVERTED, never deleted, and each inversion kills its own
positive control — so every one carries a verbatim pre-fix replica driven
through the real allocator under the same regime and asserted FIRST.

**⭐ THE REPLICA-FIDELITY CHECK is the best thing this phase found.** A replica
is only worth what its faithfulness is worth, and "I copied it carefully" is not
a measurement. Red-at-parent supplies one: revert ONLY the production hunk, run,
and the shipped body must corrupt the same way the replica does.

| site | carrier | control | fixed | red-at-parent fidelity |
|---|---|---|---|---|
| 5 | `entries` | 14 red | 0 / 28 | 14 = 14 |
| 2 | `terms` | 9 red, band (9,6,0,21) | 0 / 21 | 9 = 9, band exact |
| 7 | `values`+`keys` | 12 red / 6 clean | 0 / 18 | 12 = 12 |
| 8+9 | `terms` + `key` | red at 400, clean at 200 | ok at both | same reason string, verbatim |
| 1 | `mfa` | 2 red ON, 0 OFF | 0 / 301 both arms | **cells 253+254, cell-identical** |

Site 1 is the strongest of the five: fidelity is not a matching COUNT but the
same two argument counts, at the same predicted-window headroom (2 and 0), with
the same reason string. Site 8+9's is the weakest form — a matching reason
string rather than a matching count — because that probe is size-armed rather
than swept, and that is stated rather than dressed up.

**Sites 8 and 9 share ONE commit**, departing from per-site granularity on
purpose: both carriers live in the same loop, so one rewrite removes both and
splitting it would have landed a half-rooted loop in between. Dispositions stay
per-carrier, and they name different constructs — 8 `with_accumulator`, 9
`with_rooted`.

**⭐ `with_rooted` NESTED INSIDE `with_accumulator` WORKS, verified at the bytes.**
`rooted_push` refuses unless its handle is innermost, and the inner scope IS
innermost while open because the accumulator only pushes after it pops. That was
the one premise of the 8+9 edit not known in advance. Reusable for any site
mixing a run-carrier with a single-Term carrier.

## ⚠️ A CENSUS PREDICTION I GOT WRONG, AND THE FORM FACT UNDER IT

At sites 8+9 I predicted 40/26/14 and measured **41 raw / 26 production / 15
cfg(test)**. Measured at the parent `3ec7cf1` rather than assumed: the parent IS
40/26/14; the production S3b hit MOVED `:151 → :163` and **SURVIVES the fix**
(the match-bind is still written, now inside a rooted scope); and the new
cfg(test) hit at `:634` is **my own replica**.

⭐ **AN INVERTED PROBE'S REPLICA CAN ITSELF CARRY A CLASSED SHAPE AND MOVE THE
CENSUS. Predict the FIX's delta AND THE PROBE's.** I predicted only the fix's.
First site where it bit — the replicas at 14/5/2/7 are push loops, which no
class matches. Site 1's prediction (unchanged 41/26/15) was then exact.

⭐ And the standing form fact: **the census counts SHAPE, not DEFECT.** A rooted
site still scores. A census reduction is not this lane's success criterion in
either direction.

## 🔴 AN OPEN FINDING OUTSIDE THE RULED POPULATION — struct-field carriers

Found while reading site 1's function, MEASURED, and NOT fixed: the tranche is
ruled and **a finding does not authorize its own follow-up**.

`alloc_spawn_request` reads `request.from` and `request.group_leader` at
`:530-531`, AFTER three collecting calls. They are `Term`s in a CALLER-OWNED
STRUCT, never bound to a local. The RF-006 sweep that produced this lane's
population is **BINDING-derived** — it enumerates `let` bindings, and graded
exactly five carriers here (`args` SAFE-A, `mfa` REAL, `opt_list` SAFE-A, `op`
and `req_id` SAFE-C) — so a struct field is outside its population **by
construction**, not by an oversight within it.

**Measured** (probe kept at `gate-logs/111/struct_field_from_probe.rs.txt`, run
against the shipped body with an EXTERNAL boxed pid in `from`, which an embedder
can do because the function is public API):

- **50 red / 251 clean / 0 refused**, first red at `args 251`.
- **UNCHANGED at 50/251 after `mfa` was rooted** — in the SAME run where site
  1's own band went from 2 red to 0 red / 301 clean on both arms.

That second line is the whole attribution. The reader looks at slot 2 while site
1's carrier is slot 4, so downstream damage was the live alternative; the
instrument is demonstrably sensitive to a fix, and this carrier did not move.
**`from` is an independent carrier, not collateral.**

⛔ **WHAT THIS IS NOT.** It is ONE measured instance. Whether other struct-field
carriers exist anywhere else is **UNMEASURED**, and I am not extrapolating from
one site to a population claim — that is the exact move that produced this
lane's earlier retraction. The claim is: this function, these fields, red.

The probe is deliberately NOT committed as a test: it asserts nothing, and a
test pinning a KNOWN-OPEN defect would have to be rewritten the moment the
defect is ruled and fixed. Routed to the lead for a ruling on whether the
struct-field shape enters the population.

# TRANCHES 1 AND 2 COMPLETE — 13 sites rooted, probes inverted

Pin `ea7211a`, 15 commits ahead of `origin/main` `8d64fd3`. Nothing pushed,
nothing tagged. Phase 4 (the seal) NOT started — it is held on Cally's §8.2,
per the remedy proposal's own clause *"until ruled: no sink signature changes"*,
which binds phase 4 and nothing earlier.

## The sites

Tranche 1 (native): 3, 14, 5, 2, 7, 8, 9, 1, 10, 11, 15.
Tranche 2 (`beamr-wasm/src/convert.rs`): 12, 16.
⛔ Site 4 untouched, probe still green, never inverted. Site 6 open — no
serialised leg fell out naturally; Waffles has ruled it to after the seal.

One commit per site, except 8+9, where a single loop carries both defects and
one edit fixes both — disclosed in that commit rather than quietly merged.

## What the inversions are worth

**Every probe was INVERTED, never deleted, and every one carries a synthetic
positive that survives the fix.** Without that, a post-fix green is the
asleep-instrument reading: the same value whether the site is safe or the sweep
stopped applying pressure.

**REPLICA FIDELITY, cell-identical at five sites and not merely count-matched:**

| site | the control and the reverted production body agreed on |
|---|---|
| 1 | args 253 + 254, at headroom 2 and 0 |
| 10 | 5 = 5 corrupt, 31 = 31 clean |
| 11 | element 635, same reason string |
| 15 | `entry 16: key is not a binary — carrier went stale`, same string |
| 12 + 16 | the whole band `(21, 27, 16, 8)`, cell for cell |

That is what makes each control *calibrated* rather than merely plausible: the
replica is not a body that also fails, it is a body that fails in the same place
for the same reason.

## Two hazards tranche 2 had that the native sites did not

Both **measured**, neither argued from the shape of the code.

1. **Recursion into an open accumulator scope.** `json_value_to_term`'s Array
   arm recurses into itself, so a nested array opens an accumulator scope inside
   an open one — and `rooted_push` refuses unless its handle is innermost. The
   existing sweep is structurally incapable of testing this: its array elements
   are strings, so it never recurses.
   `ar1_site12_nested_arrays_open_an_accumulator_inside_an_open_one` drives
   arrays of arrays at 3 shapes × 4 margins through both arms. The replica goes
   red on those cells — which is the part that matters, because it proves the
   fixture applies real collection pressure rather than being quietly unpressed
   — and the shipped body is clean on every one. A refusal is scored there as
   its own named outcome, since a refusal is exactly what a broken nesting
   produces and the flat sweep counts refusals as "not corruption".
2. **Detachment.** Every production caller in that file is detached, and
   `with_rooted` returns `badarg` with no process attached. Had
   `with_accumulator` not fallen back to owned storage, this fix would have
   turned the entire production path into a refusal — a behaviour change wearing
   a fix's clothes. The tier-2 test now runs both arms detached and asserts the
   shipped one clean, on every run, rather than inheriting the guarantee from
   the accumulator's own unit test.

## The instrument finding that outranks the site count

`crates/beamr/src/term/json.rs` is behind the **non-default `json` feature**.
`cargo ... --features encode` does not compile it. Every per-site `--lib` and
clippy leg run before site 11 was **structurally blind to sites 11 and 15** —
greens from legs that had never compiled the code. Denominator 1855 with the
feature on against 1838 without.

⭐ **Caught by the DENOMINATOR** — a filtered run that matched 0 tests where I
expected 1 — **not by any failure.** Canon's two all-features legs do reach the
file, so nothing shipped uncovered; the hole was in the per-site legs. Sites 11
and 15 were re-run on `--features encode,json`.

## Census predictions, and the two that missed

Sites 11, 1, 10, 15 and 12 hit their pre-registered census predictions exactly.
The two misses are worth more than the hits:

- **Sites 8+9** — predicted 40/26/14, measured 41/26/15. Measured at the real
  parent rather than assumed: the parent IS 40/26/14, the production S3b hit
  MOVED `:151 → :163` and survives the fix, and the new cfg(test) hit at `:634`
  is my own replica. ⭐ **An inverted probe's replica can itself carry a classed
  shape.**
- **Site 16** — predicted 42/23/19 → 41/22/19, measured unchanged. Production
  S3e fell 8 → 7 exactly as predicted, and production S3b rose 11 → 12, because
  **the remedy itself** introduces `let key_term = match
  context.alloc_binary(...) {`, a let-bound match with an ALLOC call inside the
  hunt's window — the literal S3b definition. ⭐ **The fix can add a classed
  shape too. Predict the removal, the fix's own addition, and the probe's.**
  And that hit has **zero exposure**: the key is pushed on the very next line.
  The census counts SHAPE, NOT DEFECT, and the remedy manufactured a production
  false positive by construction.

## Disclosed, not resolved

`sort_pairs_by_key` (sites 15, 16) sorts by RAW TERM VALUE exactly as the
pre-fix `sort_by_key` and `alloc_sorted_map` did — but on pointers that are live
rather than possibly stale, so key ORDER can differ from pre-fix. This does not
settle the ground pack's sibling ordering-by-raw-value hazard; it **inherits**
it, on valid pointers.

`alloc_sorted_map` stays in the wasm crate: `object_to_term`, the JsValue path
(id 17), is a separate crossing outside these tranches and is still its caller.

# PHASE 3 — THE FULL CANON BATTERY, PRE-REGISTERED AND RECONCILED

Pin `ea7211a`. Prediction written and committed to
`gate-logs/111/BATTERY-PREDICTION.md` **before the runner was launched**, with
its own stop-conditions stated up front. Raw logs and per-leg rc at
`gate-logs/111/battery/battery-ea7211a.{tsv,legN.log}`.

## The reconciliation — three predicted legs, three exact hits

⭐ **AXES NAMED, per the rider:** result-lines · passed · failed · ignored.

| leg | predicted | measured | |
|---|---|---|---|
| `wasm-tests` | 2 / 83 / 0 / 0 | 2 / 83 / 0 / 0 | ✅ |
| `tests` | 76 / 2144 / 0 / 0 | 76 / 2144 / 0 / 0 | ✅ |
| `tests-all-features` | 76 / 2154 / 0 / 0 | 76 / 2154 / 0 / 0 | ✅ |

All 8 legs rc 0. `SCORED == DECLARED == 8`. Runner marker **COMPLETE**, derived
from the count and a stable pin rather than from an exit code — the exit code is
not the verdict.

Pin identical at open and close: `ea7211a390c1869d06297d98b4cdd46386086033`.

The `+3 / +3 / +1` deltas were derived at the bytes with `git grep -c` against
both refs rather than read off a diff, because this repo renders diffs
side-by-side and a `^+`-anchored grep silently returns zero — which would have
made the whole delta look like nothing at all. The `+3` is `accumulator.rs`'s own
unit tests (phase 1, a file that did not exist at the base); the `+1` is site
12's nested-scope test. **Thirteen sites, zero other new test functions: every
other probe was INVERTED IN PLACE. The arms changed, the count did not.**

## ⚠️ TWO READINGS I ALMOST GOT WRONG, BOTH CAUGHT BY MEASURING

1. **A 7-row TSV is not a failed battery.** I first read the `.tsv` at 7 rows
   against 8 declared legs — literally one of my own pre-registered stop
   conditions (`SCORED != DECLARED`). It was not an abort: leg 8 was still
   running, and `ps` showed `cargo test --workspace --all-features` live with its
   log written one second earlier. ⭐ **A partial artefact and an aborted one look
   identical on disk. The process table is what tells them apart** — and the
   difference between "in flight" and "aborted" is the difference between waiting
   and raising an alarm.
2. **The runner's `tree pre: 2 → tree post: 3`** is a real delta and I did not
   pre-register it. Itemised rather than explained away: the census is
   `git status --porcelain -- . ':!.claude' | wc -l`, and the third entry is
   `gate-logs/111/battery/` going from an empty directory (which git does not
   list) to a populated one — the battery's own outputs. `git status
   --untracked-files=no` is **EMPTY**: zero tracked files moved during the run.
   The bytes that ran are the bytes that ship.

## ⚠️ THE DISCLOSED BLIND SPOT, restated because a green must be read for what it covers

Leg 5 (`tests`) runs `--features beamr/encode`, which does **not** compile
`crates/beamr/src/term/json.rs` — that file sits behind the non-default `json`
feature. **Sites 11 and 15 are invisible to leg 5 and always were.** Only leg 8
(`tests-all-features`) reaches them. This was stated in the prediction before the
run, not discovered in it, and it does not touch the `+3`: `accumulator.rs` is
not feature-gated. Canon as a whole does reach every site; the hole was in the
per-site legs I ran mid-lane, and it was caught by a DENOMINATOR — a filtered run
matching 0 tests where I expected 1 — not by any failure.

# PHASE 5 — ROW-2 ACCOUNTING, AND WHAT IT REFUSES

## Row 2, by name — 17/17 NAMED, ZERO SILENCES, and it does NOT discharge

Row 2 as amended requires each of the seventeen to be exactly one of
`SAFE-ROOTED` / `STRUCTURALLY-ELIMINATED` / `FIXED-UNVERIFIED`, with silence
about a site being the failure. At this tip:

| disposition | count | sites |
|---|---|---|
| `STRUCTURALLY-ELIMINATED` | 13 | 1, 2, 3, 5, 7, 8, 9, 10, 11, 12, 14, 15, 16 |
| `PENDING` | 4 | 4, 6, 13, 17 |

**Zero silences — and `PENDING` is not one of row 2's three permitted
dispositions.** So the honest statement is that row 2 is **NOT dischargeable at
this tip**, and the instrument says so at its own hands rather than at mine:

```
$ ledger_check.py --sign-off
  checks fired: population, schema, file-presence, fixture-markers,
                structural-evidence, sign-off-silence
⛔ REFUSED:
  SIGN-OFF: 4 crossing(s) still PENDING: [4, 6, 13, 17].
            Silence about a site is the failure
rc=2
```

`ledger_check.py` in default mode is rc 0 and consistent; `--sign-off` is rc 2.
Both run, both reported. A pass in the mode that permits `PENDING` is not a
row-2 discharge and is not offered as one.

## ⛔⛔ SITES 13 AND 17 — THE FILE THIS LANE EDITED IS HALF-FIXED

Sites 13 and 17 live in `crates/beamr-wasm/src/convert.rs` — **the same file
tranche 2 just rooted sites 12 and 16 in** — and both already have
**RED-AT-PARENT DEMONSTRATED** (by Cally Ray at `308b448`, unchanged by this
lane). They were not in the dispatched tranche and I did not touch them.

Read at the bytes at this tip, both shapes are fully present and unrepaired:

- **site 13**, `array_to_term:251` — `let mut tail = Term::NIL` reassigned
  across `value_to_term(...)`, which allocates; sink `alloc_cons`.
- **site 17**, `object_to_term:266` — `pairs.push(...)` accumulating across
  `value_to_term(property, ...)`; feeds `alloc_sorted_map`.

⇒ **`convert.rs` now has one path rooted and its twin red.** `json_value_to_term`
(the `serde_json` path, sites 12 + 16) is safe on both arms;
`value_to_term`/`array_to_term`/`object_to_term` (the JsValue path, sites 13 +
17) is red on both arms. **An embedder entering through JsValue gets nothing
from tranche 2.** This is stated plainly so that "the wasm sites are done" can
never be read off this lane.

⭐ **AND THE REASON THEY WERE PARKED HAS ALREADY EXPIRED.** The gate's row 4
deferred site 17 because a red-at-parent test needs "a wasm32 +
`wasm-bindgen-test` + node leg nobody has costed." That leg was **PRICED at
`308b448` at setup cost ZERO** — it was already installed, pinned in
`rust-toolchain.toml`, and wired as `wasm-tests` in `gates.json:7` plus CI, and
it ran green in this lane's own battery at 83 passed. **The blocker was
discharged; the sites stayed parked behind it.** That is
[[phantom-gate-law]] — a blocker that outlives its own resolution — and naming
it is not the same as acting on it: **the tranche was dispatched, so re-opening
it is the lead's call, not a follow-up my own finding authorises.**

## ⚠️ AN ADDRESS ROT THIS LANE CAUSED, MEASURED AND REPAIRED

The ledger recorded site 13 at `function_line 177` and site 17 at
`bind_line 199`. Measured at this tip they are at **251** and **273**.

Attributed rather than assumed, by reading both ancestors:

- At the ledger's own registering commit `1f7e675`: `array_to_term` at 177,
  `object_to_term` at 192.
- At this lane's base `8d64fd3`: **identical, 177 and 192.**

⇒ The addresses were correct until this lane, and **the drift is entirely
mine.** `convert.rs` grew 848 → 1167 lines because tranche 2 rooted sites 12
and 16 *elsewhere in the file*, so **two sites the lane deliberately did not
touch had their addresses invalidated by the lane anyway.** Their bodies are
**BYTE-IDENTICAL** between `8d64fd3` and this commit, verified by direct
comparison, so nothing about the sites themselves changed.

Shift measured, not inferred: `177 → 251` and `192 → 266`, **both exactly +74**,
and corroborated by a second instrument — `shape_hunt.py` flags site 17's
carrier at `:273`, which is `266 + 7`, the same offset the original
`199 = 192 + 7` carried.

**REPAIRED** by re-pointing the five affected line fields, preserving the
originals in an `address_erratum` on each row, following the precedent the
ledger's own `function_erratum` already set. **No disposition and no count was
touched** — `--sign-off` refuses the same four sites, before and after.

⭐ **THE INSTRUMENT GAP UNDER IT.** `ledger_check.py` machine-verifies the
replacement site of a `STRUCTURALLY-ELIMINATED` row — that is the
`structural-evidence` check, and it is exactly right. **Nothing verifies the
address of a `PENDING` row.** `file-presence` passed identically before and
after the repair, because it checks the file, not the line. So **the rows most
likely to rot are the only rows with no check on them**, and this lane just
demonstrated the rot by causing it. Recommending the check be added; not adding
it, because the script is a committed shared instrument other people's numbers
cite.

## Shape-hunt re-run, WITH CONTROLS

```
=== TOTALS: 42 raw · 23 production · 19 cfg(test) ===
population walked: 349 files
PASS S3a · PASS S3b · PASS S3c · PASS S3d · PASS S3e
ALL 5 CLASS CONTROLS PASS
```

Unchanged from the site-16 measurement. The controls matter more than the
totals: every class was shown **this run** to be able to produce a presence, so
a low count is a measurement rather than a silence. `cfg(test)` hits are
deliberately not filtered — a hunt that hides its own control certifies
nothing — and production still carries `convert.rs:273`, which is site 17,
correctly visible and correctly unfixed.

# TRANCHE 3 — SITES 13 + 17, THE JsValue PATH

Re-opened by the lead's ruling after this lane's own finding that `convert.rs`
was left HALF-FIXED: tranche 2 rooted the `serde_json` path (sites 12, 16) while
its twin the JsValue path (sites 13, 17) stayed red on both arms. Scope is
exactly those two sites; nothing else rides.

## ⛔ THE PRECONDITION, AND IT IS THE MOST IMPORTANT PART OF THIS TRANCHE

The red-at-parent for these sites was demonstrated by Cally Ray at `308b448`.
By the time the tranche re-opened that evidence was **sixteen commits and one
landed tranche old**, and the file had grown 848 → 1167 lines in between.

⭐ **A RED THAT PREDATES THE TREE YOU ARE FIXING IS NOT A RED AT THE TREE YOU ARE
FIXING.** So her two probes were applied **verbatim** to the ship tree
`ae29d2c` and re-run BEFORE either site was touched:

```
test result: FAILED. 83 passed; 2 failed; 0 ignored
  site 17 probe: JsValue("failed to allocate map term")
  site 13 probe: "converted JavaScript array is a proper list"
```

Transcript: `gate-logs/111/tranche3/red-at-ae29d2c.log`.

**The 83 is exactly the pre-existing baseline**, so the two failures are
attributable to the probes rather than to collateral breakage — the denominator
is doing the attribution, not my reading of it. Both failure modes are
stale-pointer shaped (a corrupt cons chain; a map allocation that cannot be
built from moved keys) rather than allocation-exhaustion shaped.

After the fix, at the same tree: **85 passed / 0 failed**, `wasm32-check` rc 0.
Transcript: `gate-logs/111/tranche3/green-after-fix.log`.

## The two remedies

Both mirror their already-landed twins exactly.

- **Site 13** `array_to_term` — the threaded tail, same carrier as sites 11 and
  12. Same direction flip as site 12: the pre-fix body walked `.rev()` and
  prepended, the remedy walks forward and appends. **Same list order**; what
  changes is allocation order. `depth` is passed through unchanged — the depth
  wall belongs to `value_to_term` and this remedy does not move it.
- **Site 17** `object_to_term` — the `Vec<(Term, Term)>` of boxed pairs held
  live across BOTH `alloc_binary` and the recursive `value_to_term`. Remedy is
  site 16's: accumulate, `sort_pairs_by_key`, `to_map_pairs`.
  ⚠️ That sorts by RAW TERM VALUE exactly as `alloc_sorted_map` did, but on
  pointers that are LIVE rather than possibly stale — it **inherits** the ground
  pack's ordering-by-raw-value hazard on valid pointers rather than settling it.

## A comment that outlived its fact, corrected rather than overwritten

Site 16's landed commit carries the line *"⛔ `alloc_sorted_map` STAYS:
`object_to_term` (the JsValue path, a separate crossing) is still its caller."*
**True when site 16 landed, false the moment site 17 was rooted.** It is left in
place as an explicit correction rather than silently replaced, because the fact
it asserted is exactly the kind a later reader would rely on.

`alloc_sorted_map`'s last PRODUCTION caller is gone. It now sits at file scope
behind a `#[cfg(test)]` gate, because **two sibling test modules need it** — the
sites 13/17 replicas and the sites 12/16 one — and duplicating a control's
helper is how two copies drift apart. My first attempt put it inside one test
module and the compiler refused it from the other; the error was the useful part.

## The inversion, and why the probes could not be left as they were

Cally's probes drive PRODUCTION `value_to_term`. **The remedy that makes them
pass also destroys them as evidence** — a single-armed green cannot separate
"the accumulator survived a move" from "no move happened" or "the input stopped
pressing". So they are INVERTED into two-armed tests carrying
`value_to_term_unrooted_replica`, which holds sites 13's and 17's PRE-FIX bodies
verbatim.

⭐ **The replica recurses into ITSELF, not into production.** A replica that
recursed into the fixed body would be pre-fix only at the top level and would
quietly stop pressing at depth — a control that is unpressed everywhere except
its first frame.

Each test asserts the shipped arm Clean on every cell **and** that the control
was non-Clean on at least one, so a control that stops pressing fails loudly
instead of certifying by silence. A refusal is scored as its own named outcome,
never folded into "clean" — a broken rooting can produce either.


## ⭐⭐ THE CONTROL FAILED, AND IT WAS RIGHT TO — THE FINDING IS THE INSTRUMENT'S

The first two-armed run went red on my own per-cell positive control, not on
either arm's result:

```
heap never grew (466 -> 466) at count 40 -- this cell applied NO collection
pressure, so neither arm's result is evidence
```

The obvious reading is "the input is too small", and I acted on it: the object
cells were raised 40 → 60. **The rerun reported the identical `466 -> 466`.**
That non-movement is the whole finding — a threshold problem moves when you move
the threshold.

A discriminating diagnostic (print the arm and whether the body returned Ok)
settled it rather than leaving it to inference:

```
heap never grew (466 -> 466) at count 60 arm CONTROL built_ok=false
```

⭐⭐ **`after > before` ON `total_capacity()` WITNESSES A HEAP *RESIZE*, NOT A
*COLLECTION*.** The pre-fix object replica REFUSES — `failed to allocate map
term`, exactly the stale-key signature this tranche is about — and it refuses
*earlier than the heap ever resizes*. So a resize-based pressure guard scores a
correctly-failing control as an unpressed cell. **The guard was mis-grading the
one arm it exists to grade, and it was doing so by being right about the wrong
quantity.** There is no collection counter anywhere on the heap to ask instead:
`total_capacity()` (`crates/beamr/src/process/heap.rs:384`) is the only
observable.

### The remedy, and why it is not a weakened guard

A guard that fails an honest run is usually deleted. This one is **conditioned
instead, on the outcome it is grading:**

> A `Refused` or `Corrupt` outcome is **self-evidently pressed** — the body
> failed. Only a `Clean` outcome can be bought by an input too small to press
> anything. So the pressure witness is required exactly where a false clean is
> possible: on `Clean`, **on either arm**.

This is strictly stronger where it matters and honest where it did not apply. A
shipped arm that comes back Clean without a witnessed resize still fails loudly
— and because the leg is now green, every `Clean` the fixed arm returned has a
measured resize standing behind it. That is not read off the log; it is what
the passing assert *means*.

⚠️ Stated plainly so no later reader over-claims it: these controls infer
collection pressure from **resizes**, because the heap exposes nothing else.
That limitation is inherited, not introduced here, and it is now load-bearing
across probes this lane did not write. Recorded as a finding; **a finding does
not authorise its own follow-up**, so it is routed rather than acted on.

### ⛔ THE SAME FLAW, INVERTED, IN THE PROBE THIS TEST DESCENDS FROM

Cally's original site-17 probe could never have surfaced any of this. Its
`.expect()` sits **inside** the scope, so on a refusal it panics **before its own
heap-grew assert ever runs**. The control was unreachable in precisely the case
it was built to grade, and had looked green by never executing.

The ordering rule adopted here is that inversion: **grade the refusal on its own
terms first, then let the pressure witness guard the success path** — the only
path where a false clean can hide. The nested-scopes test was re-ordered to
match, so all three cells now obey one rule rather than two.

**This is the asleep-instrument family caught twice in one tranche — once in an
inherited probe, once in my own replacement for it. Both times a guard rather
than a failure did the work.**

## The expired blocker, in the words it earned

The evidence that these sites were red was real, and it was **sixteen commits and
one landed tranche stale** by the time the tranche re-opened. Left unexamined it
would have done one of two things: blocked the fix as unproven, or licensed it on
a red that no longer described the tree. **A blocker that outlives its own
resolution holds ruled work while looking like diligence — re-measure the
BLOCKER, not the work.** Re-measuring it took one leg and produced a red at the
ship tree with the baseline intact as its denominator.

## THE LEDGER, AND THE COUNT MOVEMENT STATED OUT LOUD

Rows 13 and 17 move `PENDING` → `STRUCTURALLY-ELIMINATED`, each carrying a named
replacement construct and a `file:line` that **`ledger_check.py` verifies at the
bytes**. That is the whole difference between a disposition and an assertion:
the script re-reads the file and refuses if the construct is not on that line.

⭐ **THE REFUSAL COUNT MOVES, AND THE INSTRUMENT SAYS SO ITSELF:**

```
BEFORE   ⛔ SIGN-OFF: 4 crossing(s) still PENDING: [4, 6, 13, 17]
AFTER    ⛔ SIGN-OFF: 2 crossing(s) still PENDING: [4, 6]
```

`--sign-off` still exits **rc 2**, and it should: row 2 does not discharge while
any crossing is silent. What changed is the size of the silence, from four sites
to two, and the two that remain are named. Sites 4 and 6 were already answered as
outside this tranche — both `verification_leg: native`, neither citing the wasm
leg — so nothing here is waiting on them by accident.

`--self-test` passes **10/10 with the STRUCTURALLY-ELIMINATED arm firing in BOTH
directions**: it refuses a claim that is false at the bytes, and accepts one that
is true. Either arm alone would be satisfied by a checker that always says the
same thing.

### ⭐⭐ THE CHECKER CAUGHT A ROT I WAS NOT LOOKING FOR — IN ROW 16

Running it turned up an unrelated refusal:

```
site 16: replacement construct 'with_accumulator' is NOT present at
crates/beamr-wasm/src/convert.rs:202 -- the claim is refuted at the bytes
```

Traced rather than patched. Nothing about site 16's remedy changed. The five
lines that moved it 202 → 207 were added **directly above it, by this tranche's
in-place correction of site 16's own "`alloc_sorted_map` STAYS" comment** — the
comment that stopped being true the moment site 17 was rooted.

**A DOCUMENTATION EDIT MOVED A MACHINE-VERIFIED ADDRESS.** And the asymmetry is
the part worth keeping:

| field | checked at the bytes? | how its rot behaved |
|---|---|---|
| `replacement_site` | **yes** | announced itself as rc 2 the same run, naming row and line |
| `function_line` / `bind_line` | no | rows 13/17 drifted **+74** in silence; a human had to notice |

Same class of rot, same file, same lane, same day — **caught instantly where an
instrument was watching, and only by luck where none was.** That is an argument
for extending the check to the pre-fix addresses, which is a change to Cally's
script and therefore hers to make; it is routed, not made here.

⛔ The pre-fix addresses on rows 13/17 are **deliberately NOT re-pointed again**.
This lane's own fix moved their enclosing functions once more (`array_to_term`
251 → 256, `object_to_term` 266 → 294), but the shape those addresses name no
longer exists. They are historical pins to `ae29d2c`, the tree where the pre-fix
body was last measured red. **Re-pointing an address for a shape that is gone
would be inventing one.** Rows 12 and 16 were left the same way.
