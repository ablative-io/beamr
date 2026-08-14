# Pre-registered prediction — #78 battery at 70ce683

Written 2026-08-15, BEFORE reading any leg output. Priors from
gate-logs/87 (battery at b31375a, measured this session; evidence commit
27220aa docs-only since, 70ce683 adds one test to the EXISTING
jit_wireup binary).

Axes NAMED: result-lines / passed / failed / ignored.

| leg                | prior (b31375a)   | predicted (70ce683) | delta     |
|--------------------|-------------------|---------------------|-----------|
| tests              | 75 / 2115 / 0 / 0 | 75 / 2116 / 0 / 0   | +1 passed |
| tests-all-features | 75 / 2125 / 0 / 0 | 75 / 2126 / 0 / 0   | +1 passed |
| all other 6 legs   | rc 0              | rc 0                | none      |

Result-lines derivation (committed BEFORE the battery starts): my axes
counter counts `test result:` summary lines — one per test BINARY (the
awk over grep used at #87's battery). The pin joins the EXISTING
jit_wireup binary, so binary count and result-lines are UNCHANGED at 75;
only passed moves, +1 both legs. (#104/#106 each moved result-lines +1
because each added a NEW one-test binary — consistent with per-binary
counting, not evidence for per-test counting.)

Confirmation requires the test NAME in each leg's own output.
Marker required: COMPLETE 8/8, pin stable at 70ce683, tree census 0 both
ends (excluding .claude).

## Re-registration for battery 2 at 69a7df2 (written BEFORE any leg output)

Battery 1 at 70ce683 went RED: legs 2 + 7 (both clippy) rc 101 on
clippy::assertions_on_constants at the pin — the runner's COMPLETE marker
derives only from scored==declared + pin stability, so the per-leg rc
table was the verdict (the #84 marker-vs-verdict class, caught at the
tsv). Test legs matched prediction exactly (75/2116/0/0, 75/2126/0/0,
pin by NAME both legs). Red run kept in evidence.

Erratum (mine): the pre-battery check ran the full clippy leg command
BEFORE the pin test existed and only --example after — the check must
re-run the leg's actual command after the LAST edit.

Prediction at 69a7df2 (doc/attr-only delta on the pin): identical axes —
tests 75/2116/0/0, all-features 75/2126/0/0, pin test by NAME both legs,
all 8 legs rc 0, pin stable at 69a7df2, census 1/1 disclosed (same
untracked falsifier log, identity unchanged).
