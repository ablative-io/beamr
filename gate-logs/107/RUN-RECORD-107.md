# CRASH-2 RUN RECORD — the nested-run handler leak, fixed and gated

Artemis Peach. Fix commit **`4f27c6c`**, cut from `1965650`. Both batteries ran
on the working tree at pin `1965650` carrying exactly the bytes that commit
contains.

## Verdict

**Battery 2: COMPLETE 8/8, every leg rc 0.** Per-leg tsv is the verdict
(`battery2-crash2.tsv`); the runner's COMPLETE line is corroboration, not the
claim.

| axis | prior (#95 @ `c9a63c6`) | predicted | measured |
|---|---:|---:|---:|
| leg 5 `tests` result-lines | 75 | 76 | **76** |
| leg 5 passed | 2123 | 2128 | **2128** |
| leg 5 failed / ignored | 0 / 0 | 0 / 0 | **0 / 0** |
| leg 8 `tests-all-features` result-lines | 75 | 76 | **76** |
| leg 8 passed | 2133 | 2138 | **2138** |
| leg 8 failed / ignored | 0 / 0 | 0 / 0 | **0 / 0** |

**+1 result-line and +5 passed on both legs, exact** — the one new test binary
and its five arms, nothing else moved.

## Two batteries, and why

**Battery 1 (`battery-crash2.*`): RED — 7/8 rc 0, leg 1 `fmt` rc 1.** Kept as
evidence, not discarded. rustfmt wanted five call sites in the new test file
collapsed; no production byte was involved. `cargo fmt --all` applied, then the
WHOLE battery re-ran — a formatting change is an edit, and both clippy FULL legs
must run after the last edit.

**Battery 2 (`battery2-crash2.*`): GREEN 8/8**, axes above, pin stable
(`1965650` pre and post), tree 6 modified pre and post.

## My prediction MISSED on the absolute axes — erratum

Battery 1's PREDICTION.md named 74/2117 and 74/2127. The measurement was
76/2128 and 76/2138. **The prior was wrong, not the delta.** I took the baseline
from `gate-logs/103/` on the reasoning that 103 is the highest-numbered gate-log
directory, and asserted HEAD was three docs-only commits ahead of it. Measured:
`git rev-list --count 07e8a60..HEAD` = **35**, spanning six test-adding lanes.
The true prior is #95 at `c9a63c6` — one docs-only commit behind HEAD —
re-extracted at my hands from `gate-logs/95/leg5.log` and `leg8.log`.

Against the correct prior the delta prediction held EXACTLY. Full erratum in
`PREDICTION.md`. **Banked rule: pick the prior battery by COMMIT ANCESTRY, not
by the largest gate-log directory number.** Lane numbers are assigned by topic,
not by time; a lane that starts later can land earlier, and `git merge-base
--is-ancestor 07e8a60 c9a63c6` says exactly that here.

## Discrimination — measured in BOTH directions

The gate is not merely green at the fixed bytes:

| bytes | count arms (1, 2, 4) | control (3) | liveness arm (5) |
|---|---|---|---|
| unfixed `1965650` | **RED** — 7 vs 1 at K=6; 3 at K=2 vs 7 at K=6; 3 per trampoline at depth 2 | green | green |
| half 1 only (floor, no re-offer) | **all pass** | green | **RED — process killed (`got Error`)** |
| whole fix `4f27c6c` | green | green | green |

The middle row is the point of the liveness arm: a fix that stopped the leak and
made caught exceptions uncatchable would satisfy every frame count in the file.
Only the value assertion catches it.

## Falsifier rails inside the gate

Every arm asserts the death record is non-empty AND carries at least one
line-bearing interpreted frame — a reader that cannot parse the record would
otherwise report zero compiled frames and look green. Compiled arms assert the
trampolines they depend on actually reached COMPILED with every compile job
settled; the control asserts `submissions == 0`.

## A wrong pin I wrote, measured, and replaced

The depth-2 arm first asserted the interpreted middle never tiers up, on my
reasoning that `jit_call_interpreted` records no call miss. **That premise is
false**: the middle accrues misses on the driver's pre-tier-up calls, while the
outer is still running as bytecode, and it does compile. The claim I actually
needed — that two separate nestings exist — is now pinned on execution instead
of on compile state: the middle's compiled code contributes **zero** frames to
the record, because once the outer is compiled the middle is entered as
BYTECODE. Measured, both K values. A compile-state proxy was replaced by the
measurement it was standing in for.

## Files

- `EXIT-PATH-CENSUS.md` — Waffles' ruling-1 price: every exit from the guarded
  region enumerated with its restore named, by a mechanical instrument that
  locates the region by its own bytes. Region is one line; 0 returns, 0 `?`; 15
  of the function's 15 returns are before the install, 0 after the restore.
- `PREDICTION.md` — pre-committed prediction, the erratum, and battery 2's
  prediction committed before that run.
- `SPECIMEN-READ-1.md`, `PROBE-RECORD-GHOST.md` — the strand-A evidence chain
  that localized this seam, including the ARM C addendum refuting the corruption
  leg for the tail shape.
- `battery-crash2.*` (RED, leg 1), `battery2-crash2.*` (GREEN 8/8).

## What this does and does not close

Closes: the leak, the K+1 trace corruption, and the uncatchable-exception hazard
the fix's first half would have introduced.

Does NOT close: the aion-side identity of the six caught raises in the soak's
two-minute window. Per the ARM C addendum the soak's first caught badarg
PREDATES any leak, so the non-integer was already in the value and **the cause
of that badarg is upstream of this defect**. What this fix removes is the leak
and the false trace that made the specimen unreadable.
