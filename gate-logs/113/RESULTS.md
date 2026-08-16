# #113 AR-1 CONTROL RETROFIT — RESULTS

Successor to #112. The observable was built there; this re-points the three AR-1
controls that were reading the wrong thing.

## What was re-pointed

| control | from | to |
|---|---|---|
| `assert_pressed` (AR-1 site 13, `array_round_trip`) | `Heap::total_capacity()` | `Process::gc_attempts()` |
| `assert_pressed` (AR-1 site 17, `object_round_trip`) | same | same |
| the inline guard in `ar1_sites_13_17_nested_scopes_open_inside_one_another` | same | same |

All three live in `crates/beamr-wasm/src/convert.rs`. `crates/beamr` is untouched
by this lane.

**`gc_attempts`, not `gc_completions`.** What breaks an unrooted carrier is a
collection ENTERED and moving objects; one that fails partway has still moved
them. Completions and capacity both ride in the failure message, capacity
explicitly labelled **NOT the witness**, so a future reader is shown what the
retired proxy would have said rather than left to re-derive why it was wrong.

## ⭐ The conditioning was LIFTED, and that is the substance

The old guard fired only on `Outcome::Clean`. That condition existed for exactly
one reason: the proxy mis-scored refusals, so demanding a resize unconditionally
would have failed the arm the control exists to grade.

Run **unconditional** under the real observable — demanded on every cell, both
arms, every outcome — **86 passed / 0 failed**. Every cell collects, refusals
included.

So the condition's only reason is measured dead, and it is removed rather than
carried "just in case". The rebound control is **strictly stronger** than the one
it replaces: it grades every cell, not only the clean ones. A weakening whose
reason has died is exactly the shape that survives as an unexplained hole.

## ✅ Falsifiers — the load-bearing evidence for this lane

Predictions at `gate-logs/113/FALSIFIERS-PREREGISTERED.md`, transcript at
`gate-logs/113/falsifiers.log`.

| mutation | predicted | measured |
|---|---|---|
| **M1** remove `note_gc_attempt()` from `minor::collect` only | all three RED | **ONE of three.** site17 red; site13 and nested stayed green |
| **M1b** remove it from `minor` **and** `major` | all three RED | **all three RED** ✅ |
| **M2** shrink the site-13 cell to `count = 1` | site13 RED | **site13 RED** ✅ |

⚠️ **M1 MISSED, and the miss was named in advance.** The pre-registration said:
*"this assumes the wasm conversion path collects minor, not major. If the majors
are what run, M1 fires on fewer than three and the right response is to record
the miss and re-aim at `major::collect` — not to call a partial red a pass."*
That is what happened and what was done. Site 13 and the nested cell reach
**major** collection, whose counter was still intact.

**M2 is the arm that matters.** M1b only shows the guard notices a broken
counter. M2 shows the guard is **not vacuously true** — a real conversion can
fail to collect (`0 -> 0 attempts` at count 1) and the guard says so. Without it,
a future shrink of these inputs would sail through a guard that always says yes.

All mutations reverted; tree re-verified green at the bytes (86 passed) with the
tracked census showing `convert.rs` alone and both gc files byte-identical to
HEAD.

## ⭐ The datum that fell out

site 17, CONTROL arm, count 60: **`0 -> 1 completions` with capacity `466 -> 466`.**

That is the #112 mechanism caught in the act on the exact cell that produced the
finding. A collection **completed** and the heap **never resized** — because
`ensure_space` collects first and grows only if the collection was not enough
(`gc/mod.rs:154` then `:162`). The cell was under collection pressure the whole
time. The retired proxy read `466 -> 466` and called it unpressed.

It was never a threshold problem, and no amount of raising the input would have
fixed it. The 40 → 60 non-movement recorded in #111 was the right tell read
correctly.

## Ledger

`ledger_check.py` re-run after the edits: `--self-test` **10/10, every check
fired**; `--sign-off` still refuses with **`2 crossing(s) still PENDING: [4, 6]`**
— unchanged from #111 and correct. The machine-verified `replacement_site`
addresses sit above the edited region and did not move.

## The battery

Pin `4a66232`. Prediction committed **before** the runner, with the pin wrinkle
declared **up front**.

| leg | predicted | measured | |
|---|---|---|---|
| 4 `wasm-tests` | 2 / 86 / 0 / 0 | 2 / 86 / 0 / 0 | ✅ unchanged |
| 5 `tests` | 76 / 2148 / 0 / 0 | 76 / 2148 / 0 / 0 | ✅ unchanged |
| 8 `tests-all-features` | 76 / 2158 / 0 / 0 | 76 / 2158 / 0 / 0 | ✅ unchanged |

8/8 rc 0 · `SCORED == DECLARED == 8` · marker **COMPLETE** · pin identical at
open (`4a66232e1185b35188b33f60e73212fbb4730ffc`) and close ·
`--untracked-files=no` **EMPTY**.

Prediction pin `f47c514` vs battery pin `4a66232`: one commit, `--numstat`
**67/0, zero Rust**.

⭐ **And the prediction said in advance that this battery is the weak evidence
here.** All-unchanged cannot be confirmed by movement, only by absence of it,
which a broken runner also produces. What carries this lane is the falsifier set
above — three controls RED under M1b, site 13 RED under M2. The battery's job was
only to show the re-pointing broke nothing else, and it did that.

## ⚠️ Disclosure — the raw census moved, and it was ME

`tree pre: 16` → `tree post: 17`. **The extra line is my own doing**: I wrote
`gate-logs/113/RESULTS.md` into the repo *after* the runner had already sampled
`tree pre`, so an untracked file appeared mid-run for a reason that has nothing
to do with any gate.

Recorded rather than explained away. The census that carries evidence is the
**tracked** one (stop condition 4), and that one is empty at close. But a raw
count that moves during a battery is exactly the kind of thing that should never
be left for a reader to reconcile on their own, and the honest reconciliation is
that the instrument was fine and the operator was untidy.

Note this is the mirror of #112's finding: there a flat count concealed nine
written files; here a moved count means one irrelevant file. **Neither direction
of that count means what it looks like — which is why the tracked census is the
one with teeth.**

