# Pre-registered prediction — #87 landing battery at b31375a

Written 2026-08-14, BEFORE reading any leg output. Priors from
gate-logs/106 (battery at bb96894, landed c524550 — evidence + example
probe only since; the probe is an explicit non-test target the battery
never runs).

Axes NAMED: result-lines / passed / failed / ignored.

| leg                | prior (bb96894)   | predicted (b31375a) | delta |
|--------------------|-------------------|---------------------|-------|
| tests              | 75 / 2115 / 0 / 0 | 75 / 2115 / 0 / 0   | none  |
| tests-all-features | 75 / 2125 / 0 / 0 | 75 / 2125 / 0 / 0   | none  |
| all other 6 legs   | rc 0              | rc 0                | none  |

The example adds NO test binary and NO ignored tests (the ruled landing
shape's whole point): both test legs' axes carry unchanged, ignored stays
0. Test legs compile the example (cargo test builds examples) — build
succeeds per the pre-battery clippy + cargo run confirmation.

Marker required: COMPLETE 8/8, pin stable at b31375a, tree census 0 both
ends (excluding .claude).
