# Pre-registered prediction — #104 battery at 088a4d1

Written 2026-08-14, BEFORE reading any leg output of the battery about to run.
Priors from gate-logs/102 (battery at 94856f3, landed 6167f74 — docs/evidence
only since, no test-affecting commits between).

Axes NAMED: result-lines / passed / failed / ignored.

| leg                | prior (94856f3)   | predicted (088a4d1) | delta      |
|--------------------|-------------------|---------------------|------------|
| tests              | 73 / 2113 / 0 / 0 | 74 / 2114 / 0 / 0   | +1 line,+1 |
| tests-all-features | 73 / 2123 / 0 / 0 | 74 / 2124 / 0 / 0   | +1 line,+1 |
| all other 6 legs   | rc 0              | rc 0                | none       |

UNLIKE #102, result-lines move: spawn_process_scaffold_only.rs is a NEW test
binary (its own result line), carrying exactly one test
(spawn_process_on_a_real_module_dies_on_the_landing_pad_while_spawn_runs_it).
It compiles under both leg feature sets (uses only default-feature API).

Confirmation requires the test NAME in each leg's own output.

Marker required: COMPLETE 8/8, pin stable at
088a4d1e0e9df48de720a529b27d7ee214803eca, tree census 0 both ends.
