# beamr #97 GATE-CHECKER-CONTROL — battery receipt

Base `66bcb7f` (= `origin/main`). Runner beside this file; six canon legs read
from the committed `gates.json` at run time, never transcribed.

## Verdict: 6/6 rc 0. COMPLETE marker DERIVED (legs_declared=6, legs_scored=6).

Tree check: **3 pre, 3 post**, unchanged across the run. Interpreter logged:
`/usr/bin/python3`, Python 3.9.6. Axes **73 / 2110 / 0 / 0** — unchanged from
#95's base, and that was the prediction: this lane adds no Rust test.

## The method — not mine

⭐ **"Would this instrument still report zero with its checker deleted? If yes,
it was never measuring."** Vesper's formulation, adopted as this sweep's method.

I had performed exactly that procedure by accident one lane earlier — #95's
falsifier arm 2 deleted the guard and the round-trip ratchet still reported
"105 fixtures, 0 refused" and passed. **I recorded a specific result and did not
notice I was holding a general procedure.**

## The defect — measured on the bare command, three arms

The `blocking-call-in-native-bif` leg was `ast-grep scan … --json`. Its passing
verdict is empty findings + rc 0. Measured, the same verdict is returned when:

| arm | condition | result |
| --- | --- | --- |
| **A** | rule's pattern list emptied | `[]`, **rc 0** |
| **B** | **scan path does not exist** | `[]`, **rc 0** — notice goes to **stderr**, exit code stays 0 |
| C | rule *file* missing | rc **6** — already safe |

⚠️ **Arm B is the live hazard.** `crates/beamr/src/native/` is a hard-coded path
and beamr has restructured `src/` directories twice recently (#67, #70). A
renamed or split `native/` would have left this gate scanning nothing and
reporting clean **forever**, with its only complaint on a stream whose contents
change no exit code.

⚠️ Note what the leg's own committed note already worried about: piping, and
zero-indexed line numbers. **Someone had already reasoned about this leg's exit
fidelity and missed both of the bigger holes.** Care applied to the wrong layer.

## The fix — the gate proves its checker can fire before believing a zero

`scripts/gate-blocking-call.sh`, wired in as the leg command. Three
preconditions, each of which **RAISES rather than skips**:

1. **The checker fires on a positive control** — `.ast-grep/fixtures/`, which is
   outside every crate so cargo never compiles it. It requires **one hit per
   declared pattern** (parsed from the rule, not hard-coded), so **adding a
   pattern without adding its specimen fails the gate**. An untested pattern is
   a pattern whose zero means nothing.
2. **The scan path exists and holds ≥1 `.rs` file.**
3. Only then is an empty result believed. Counts are **parsed from the JSON**
   rather than inferred from exit codes.

Stderr is never suppressed; the control's expected "4 error(s) found" notice is
**announced on the line above it** so a reader cannot mistake it for a failure.

## Falsifier — three arms RED, control GREEN

| arm | condition | hardened gate |
| --- | --- | --- |
| A | rule emptied | **FAIL** — "declares 0 patterns — the checker is empty" |
| B | scan path renamed away | **FAIL** — "scan path does not exist … would otherwise report clean" |
| C | real `std::thread::sleep` planted in `native/mod.rs` | **FAIL** — names file and line |
| control | untouched tree | **rc 0** |

Arm C's plant was reverted and `git status` on `crates/beamr/src/native/`
re-measured at **0** before the battery ran.

⚠️ **Arm B failed for the WRONG REASON on its first attempt** — the copied
script resolved `$ROOT` relative to its own scratchpad location, so it aborted
on "rule file absent" rather than on the missing scan path. A red arm is not
evidence until the redness has the cause you claimed. Re-run with `ROOT` pinned,
and the second run failed on the intended precondition.

## Live leg output

    checking the checker: the next 'error(s) found' notice is the positive control firing, as required
    checker verified: 4 control hits / 4 patterns; scanned 86 .rs files under crates/beamr/src/native/
    OK: 0 findings, and the zero is trustworthy

## Carried invariant

`refusal.count` re-measured at **55**, unchanged from #95 — the encode guard is
still in place and still declining the same population. That number falls to
zero only when #95's `Type`-chunk fix lands, and not before.

## Scope — what this lane does NOT claim

⛔ **One leg was hardened, not six.** The other five were reasoned about, not
subjected to the deleted-checker test:

* `fmt` / `clippy` / `tests` use cargo's `--all`/`--workspace` resolution, so a
  renamed crate is still picked up — but **`clippy`'s feature blind spot was
  found only one lane ago (#95)**, which is exactly why "reasoned about" is not
  "measured".
* `tests` retains the known shape from #93: a feature-gated test binary still
  builds and runs, printing `ok. 0 passed`. The battery's derived COMPLETE
  marker and denominator catch that at battery level; **CI has no equivalent.**

Those remain open under #97, and #84 (the CI verdict step collapsing per-leg
exit contracts) is the same family.
