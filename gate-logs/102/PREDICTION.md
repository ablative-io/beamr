# Pre-registered prediction — #102 battery at 94856f3

Written 2026-08-14, BEFORE reading any leg output of the battery now running
(background task bfmmqld9h). Priors from gate-logs/103 (battery at 07e8a60,
carried unchanged through afdbcf2 — no test-affecting commits between).

Axes are NAMED (form rider from #69): result-lines / passed / failed / ignored.

| leg                | prior (afdbcf2)   | predicted (94856f3) | delta |
|--------------------|-------------------|---------------------|-------|
| tests              | 73 / 2112 / 0 / 0 | 73 / 2113 / 0 / 0   | +1    |
| tests-all-features | 73 / 2122 / 0 / 0 | 73 / 2123 / 0 / 0   | +1    |
| all other 6 legs   | rc 0              | rc 0                | none  |

The +1 in BOTH test legs is `runtime_tier_up_never_moves_the_compilation_threshold`
(crates/beamr/tests/jit_wireup.rs) — jit+threads are default features, so the
new test compiles and runs under both leg feature sets. Result-lines stay 73:
no new test binary, the test joins an existing one (jit_wireup).

Confirmation requires the test NAME in each leg's own output, not just the
count (the #103 phantom-+10 lesson: counts compared across different
populations lie silently).

Marker required: COMPLETE 8/8, pin stable at
94856f3c2064eedcd98411cbfc50d6dc9f2dd45b, tree census 0 both ends.
