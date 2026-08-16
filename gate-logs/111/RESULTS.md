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
