# RUN RECORD — #102 tune_threshold doc correction + constancy pin

Lane: #102 "tune_threshold is documented as runtime behaviour but has zero
production callers." Ruled by Waffles 2026-08-14: shape (c) — keep the knob,
correct the docs, add a positive control. The ruling's ground: this is not a
compromise between wiring and deleting — it RESTORES the settled design the
doc drifted from.

Design authority (cited per the ruling):
- B-138.json:120 (constraints): "SHALL NOT implement continuous re-tuning in
  production — tune_threshold is called after benchmark analysis, not on
  every compilation"
- JIT-001.json:148 (non-goals): "tune_threshold exists but stays unwired —
  adaptive thresholds are not this brief"
- B-138.json:103 reconciled, not contradicted: its SHALL ("document ... that
  tune_threshold may adjust it at runtime") survives in the corrected wording,
  which keeps "can adjust it at runtime" and adds WHO drives it.

Refusals ruled alongside:
- (b) delete: refused — claims-carry-their-population. The knob is public API
  (`pub mod jit`, door at `Scheduler::jit_profiler()`), beamr has no
  `publish = false` and ships released cuts, so the embedder population is
  unenumerable; "zero callers I can see" is not a population.
- (a) wire it up: refused here — it is not a wire-up but a design CHANGE
  (contradicts B-138:120's SHALL NOT) plus an instrumentation build (nothing
  in the tree measures compilation time or per-function speedup; the
  jit_comparison benches never feed the knob). Staged behind the #101
  instrumentation-lever decision, where both inputs would be designed.

## Measured ground (at afdbcf2, re-verified this lane, not inherited)

- `tune_threshold` call sites: its own 4 unit tests only. Zero production,
  zero bench, zero anywhere in the ablative stack (aion/beamr/frame/
  haematite/liminal/lys/norn; the one aion hit is a fleet-state doc
  describing this finding).
- Reachability: real. `pub mod jit` (lib.rs:37); `Scheduler::jit_profiler()`
  (scheduler/mod.rs:1974) hands out the Arc. "Undriven" is true;
  "unreachable" is not — which is why the corrected doc names the door.
- `SchedulerConfig { jit_threshold: None }` maps to `DEFAULT_JIT_THRESHOLD`
  at scheduler/mod.rs:1361-1363 (`unwrap_or`).

## The change (commit 94856f3)

- `crates/beamr/src/jit/profiler.rs`: const doc (DEFAULT_JIT_THRESHOLD) and
  `tune_threshold` fn doc corrected — embedder-drivable through
  `Scheduler::jit_profiler` after offline benchmark analysis; the shipped
  runtime never calls it; threshold constant unless an embedder intervenes.
- `crates/beamr/tests/jit_wireup.rs`: positive control
  `runtime_tier_up_never_moves_the_compilation_threshold`. Drives REAL
  tier-up at the true default path (no jit_threshold override — the test
  also pins the None → DEFAULT_JIT_THRESHOLD mapping): 1000-call module,
  heat → submit → compile success, then asserts
  `current_threshold() == DEFAULT_JIT_THRESHOLD`. Goes red the day
  production wiring starts tuning, forcing the docs and B-138:120 to be
  revisited in the same change.

## Two-arm falsifier (the pin is load-bearing)

- Arm ACCEPT: shipped bytes, test alone — ok (0.04s).
- Arm MUTATE: `tune_threshold(5_000, 2.5)` wired into
  `JitProfiler::note_success` (the exact site production wiring would use;
  models continuous re-tuning on compile completion). Exact-match surgery,
  asserted matched once. Result: rc 101, failing at precisely the constancy
  assert — "a completed production compile must not move the threshold:
  tune_threshold is embedder-driven only (B-138)". Log:
  `falsifier-mutation-arm.log`.
- Restore: verified by sha equality to the pre-arm bytes
  (88a10ac760857a8f722b99b2826e5f1d1c9d5accbd5999611c708d4b902f884f).

DISCLOSURE — repeat of a recorded hazard, second occurrence: the first
restore used `git checkout --`, which restores to HEAD, and the lane's doc
edits were uncommitted — so it reverted the fix along with the mutation.
Caught by the falsifier's own before/after sha check; doc edits re-applied
and verified byte-exact against the pre-arm sha. The SAME error is recorded
in gate-logs/103's run record ("the falsifier's first run reverted the
uncommitted fix instead of the mutation"). Rail, now standing: a falsifier
restore on a tree with uncommitted lane edits must be copy-aside
(cp to scratch + cp back), never `git checkout --`.

## Battery (canon 8-leg, at 94856f3)

Runner: gate-logs/103/battery-RUNNER.sh reused byte-identical (sha256
b43254aed3c77a7e6850d39430f57963aea42421bc51aea6bf4210169ff94c76), in-repo
per the #103 precedent. Legs read from gates.json at run time.

Marker: **COMPLETE (derived: 8/8, pin stable)**, tree census 0 both ends,
opened 2026-08-14T12:39:11Z, closed 12:49:58Z. Per-leg rc all 0
(`battery-94856f3-legs.tsv`).

Axes (NAMED: result-lines / passed / failed / ignored), pre-registered in
`PREDICTION.md` BEFORE any leg output was read:

| leg                | prior             | predicted         | measured          |
|--------------------|-------------------|-------------------|-------------------|
| tests              | 73 / 2112 / 0 / 0 | 73 / 2113 / 0 / 0 | 73 / 2113 / 0 / 0 |
| tests-all-features | 73 / 2122 / 0 / 0 | 73 / 2123 / 0 / 0 | 73 / 2123 / 0 / 0 |

+1 exact in both, confirmed by test NAME in each leg's own log
(`battery-94856f3-leg5-tests.log`, `battery-94856f3-leg8-tests-all-features.log`),
not by count alone (#103's phantom-+10 lesson).

No semver consequence: docs + one test, no API change.
