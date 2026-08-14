# RUN RECORD — #104 spawn_process doc correction + landing-pad death pin

Lane: #104 "Scheduler::spawn_process enters at ip 0 — the func_info landing
pad for any loader-produced module." Ruled by Waffles 2026-08-14: shape (c) —
correct the doc to what the function does, pin the documented death.

The doc line "Spawn a process at the beginning of a module" was not
overclaimed — it was FALSE for every loader-produced module, and true only
for hand-built scaffolds whose instruction 0 is executable code.

Refusals ruled alongside:
- (b) make it resolve an entry like spawn_in: REFUSED — spawn_process has no
  function argument, so "resolve the entry" means minting a semantic nobody
  designed; spawn_in already exists for callers who mean it.
- (a) delete: PARKED to the 0.19.0 cutter's list (with #95's half-landed
  surface): survive-as-pub / rename (spawn_scaffold) / move behind
  test-support is a breaking-release question the cutter decides together.

## Measured ground

- Structural: the loader stores the parsed instruction stream verbatim
  (load.rs:657-672), so ip 0 of a loader-produced module is whatever the
  BEAM code chunk starts with.
- Empirical (this lane, via a temporary probe test through beamr's OWN
  parser — `probe-fixture-first-instructions.log`): 5/5 real erlc fixtures
  (hello, proof, if_probe, jit_real_function, recv_after_timer) parse to
  `Label` at ip 0, `Line` at ip 1, `FuncInfo` — the multi-clause dispatch
  landing pad — at ip 2, first body at ip 3+.
- Behavioural (#98, prior lane): entry at ip 0 on a pad-leading module dies
  immediately with a catchable `error:function_clause` (`Exited(Error, nil)`).
- Census (WIDER than the task recorded): ~19 call sites across 7 integration
  test files plus scheduler unit tests — all hand-built scaffolds with a
  body at ip 0, zero production callers, zero stack callers (aion's
  same-named method wraps the label-resolving `scheduler.spawn`).

## The change (commit 088a4d1)

- `crates/beamr/src/scheduler/spawning.rs`: `spawn_process` and its
  telemetry twin documented as raw-ip-0 scaffold-only entries — naming the
  landing-pad death on real modules and pointing real-module callers at
  `spawn`/`spawn_in`.
- `crates/beamr/tests/spawn_process_scaffold_only.rs` (NEW test binary):
  `spawn_process_on_a_real_module_dies_on_the_landing_pad_while_spawn_runs_it`.
  Per the ruling's sharpening the pin asserts the SPECIFIC documented shape,
  not "died": contrast arm `spawn(proof, fibonacci, [10])` → Normal / 55
  (the module is fine; note proof.beam does NOT export main/0 — first
  attempt hit Undef); death arm `spawn_process` on the same module →
  `ExitReason::Error`, result NIL, and `take_exit_exception` formats with
  `function_clause`.

## Falsifier (the pin is load-bearing)

Committed-first, so the restore is checkout-safe — the copy-aside/committed
rail from gate-logs/102 applied on its first outing.

- Mutation: `enqueue_spawn(..., 0, ...)` → `3` in spawn_process (exact-match
  surgery, asserted matched once) — models entry resolution that skips the
  pad, the precise change the pin exists to catch.
- Result: rc 101, failing at the death assert ("ip-0 entry on a
  loader-produced module must die on the landing pad") — the pad-skipping
  entry survived into a body. Log: `falsifier-mutation-arm.log`.
- Restore: sha-equal to the committed bytes (9335e7c8…).

## Battery (canon 8-leg, at 088a4d1)

Runner: gate-logs/103/battery-RUNNER.sh reused byte-identical, in-repo per
precedent. Marker: **COMPLETE (derived: 8/8, pin stable)**, tree census 0
both ends, per-leg rc all 0. Closed 2026-08-14T13:07:23Z.

Axes (NAMED: result-lines / passed / failed / ignored), pre-registered in
`PREDICTION.md` before any leg output was read — and UNLIKE #102 the
result-line count moves, because this lane adds a new test binary:

| leg                | prior             | predicted         | measured          |
|--------------------|-------------------|-------------------|-------------------|
| tests              | 73 / 2113 / 0 / 0 | 74 / 2114 / 0 / 0 | 74 / 2114 / 0 / 0 |
| tests-all-features | 73 / 2123 / 0 / 0 | 74 / 2124 / 0 / 0 | 74 / 2124 / 0 / 0 |

Both exact, confirmed by test NAME in each leg's own log.

No semver consequence: docs + one test, no API change.
