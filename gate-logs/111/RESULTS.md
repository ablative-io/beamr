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
