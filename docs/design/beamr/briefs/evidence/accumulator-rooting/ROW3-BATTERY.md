# AR-1 ROW 3 — battery record (two batteries, tsv is the verdict)

Artemis Peach. Prediction registered and COMMITTED at `830d699`
(ROW3-RECORD.md) before either launch. Pre-battery check per the #78 law
ran BOTH clippy legs' full commands after the last edit before each
launch — rc 0 all four runs.

## Battery 1 at 830d699 — RED, leg 1 (fmt) rc 1 (`row3_battery1_RED.tsv`)

The two new tests' hand-written bodies carried rustfmt drift
(`row3_battery1_leg1_fmt.log` — rejoins/splits only, zero semantic
delta). All other 7 legs rc 0. Disclosed, not discarded: the tsv is the
verdict, and the prediction's "all 8 legs rc 0" clause MISSED here — the
axes clause was never graded against this battery. Remedy: `cargo fmt`,
committed as `d7a673f` (formatting only), full battery re-run per "bytes
that ran are bytes that ship".

## Battery 2 at d7a673f — COMPLETE 8/8 rc 0 (`row3_battery2.tsv`)

NAMED AXES, measured at my counter from the leg logs' `test result:`
lines (result-lines / passed / failed / ignored) vs the committed
prediction:

| leg                | predicted         | measured          |
|--------------------|-------------------|-------------------|
| tests              | 75 / 2120 / 0 / 0 | 75 / 2120 / 0 / 0 |
| tests-all-features | 75 / 2130 / 0 / 0 | 75 / 2130 / 0 / 0 |

EXACT both legs (+3 on the lib binary from the 2117/2127 baseline at
`b77c21a`). All three walls present BY NAME exactly once in each test
leg's log (`row3_battery2_leg5.log`, `row3_battery2_leg8.log`). Census
`git status --porcelain -- . ':!.claude'` EMPTY at launch and at read.
Runner stdout captured to its own file (the #78 header-clip guard); both
batteries ran on committed trees; this evidence enters the repo in the
follow-up commit carrying this file.
