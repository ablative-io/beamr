# Pre-registered prediction — #106 battery at bb96894

Written 2026-08-14, BEFORE reading any leg output of the battery about to run.
Priors from gate-logs/104 (battery at 088a4d1, landed ffbf642 — evidence-only
since).

Axes NAMED: result-lines / passed / failed / ignored.

| leg                | prior (088a4d1)   | predicted (bb96894) | delta      |
|--------------------|-------------------|---------------------|------------|
| tests              | 74 / 2114 / 0 / 0 | 75 / 2115 / 0 / 0   | +1 line,+1 |
| tests-all-features | 74 / 2124 / 0 / 0 | 75 / 2125 / 0 / 0   | +1 line,+1 |
| all other 6 legs   | rc 0              | rc 0                | none       |

process_vm_census.rs is again a NEW test binary (own process on purpose —
absolute census asserts are race-free only there), carrying exactly one test
(census_counts_constructed_not_yet_dropped_vms_with_their_spawned_workers).
Default-feature API only, so it runs under both leg feature sets.

Confirmation requires the test NAME in each leg's own output.

Marker required: COMPLETE 8/8, pin stable at
bb96894c44f12d9c3ff7934b859692addf5da94c, tree census 0 both ends.
